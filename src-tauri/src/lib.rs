mod auth_helper;
mod caddy_install;
mod caddy_permissions;
mod caddy_process;
mod caddy_supervisor;
mod caddyfile;
mod config_store;
mod dns;
mod hosts;
mod model;
mod node_env;
mod system_trust;

use caddy_process::CaddyState;
use caddy_supervisor::{CaddySighting, ProcessInfo};
use model::Config;
use serde::Serialize;
use system_trust::CertificateTrustStatus;
use tauri::State;

struct AppState {
    caddy: CaddyState,
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Error variant returned from `start_caddy`. Serialized with a `kind` tag so
/// the frontend can branch on the variant without string-matching the message.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StartError {
    PermissionRepairRequired { message: String, path: String },
    /// An external caddy (one we didn't spawn) is already holding the ports.
    /// `external` are non-Bellboy processes — the user must confirm before kill.
    /// `bellboy_owned` are caddies started with our Caddyfile (orphans we can clean).
    ForeignCaddyDetected {
        message: String,
        bellboy_owned: Vec<ProcessInfo>,
        external: Vec<ProcessInfo>,
    },
    Other { message: String },
}

impl StartError {
    fn other<E: std::fmt::Display>(e: E) -> Self {
        StartError::Other {
            message: e.to_string(),
        }
    }
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::PermissionRepairRequired { message, .. } => f.write_str(message),
            StartError::ForeignCaddyDetected { message, .. } => f.write_str(message),
            StartError::Other { message } => f.write_str(message),
        }
    }
}

/// Combined health snapshot — what the UI needs to render the status bar in a
/// single roundtrip. Returned by `refresh_health` and consumed by the
/// focus/refresh handlers on the frontend.
#[derive(Debug, Serialize)]
struct CaddyHealth {
    is_running: bool,
    admin_api_reachable: bool,
    sighting: CaddySighting,
}

fn build_caddyfile(config: &Config) -> String {
    caddyfile::generate(&config.sites, |host| dns::resolve_external(host).ok())
}

#[tauri::command]
fn get_config() -> CmdResult<Config> {
    config_store::load().map_err(err)
}

#[tauri::command]
fn generate_caddyfile() -> CmdResult<String> {
    let cfg = config_store::load().map_err(err)?;
    Ok(build_caddyfile(&cfg))
}

#[tauri::command]
fn caddy_status(state: State<'_, AppState>) -> CmdResult<bool> {
    Ok(state.caddy.is_running())
}

/// Persist config, rewrite Caddyfile, sync /etc/hosts, reload Caddy if running.
#[tauri::command]
async fn apply_config(config: Config, state: State<'_, AppState>) -> CmdResult<()> {
    config_store::save(&config).map_err(err)?;

    let caddyfile_text = build_caddyfile(&config);
    let path = config_store::caddyfile_path().map_err(err)?;
    std::fs::write(&path, caddyfile_text).map_err(err)?;

    let domains: Vec<String> = config
        .sites
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.domain.clone())
        .collect();
    hosts::sync(&domains).map_err(err)?;

    if state.caddy.is_running() {
        state.caddy.reload(&path).map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
async fn start_caddy(state: State<'_, AppState>) -> Result<(), StartError> {
    // Pre-flight: catch the "previous sudo run left root-owned files" case
    // before Caddy spawns and silently dies.
    if let Err(issue) = caddy_permissions::check() {
        return Err(StartError::PermissionRepairRequired {
            message: issue.message(),
            path: issue.path().display().to_string(),
        });
    }

    // Pre-flight: another caddy already holds :80/:443/:2019. Auto-clean orphans
    // we recognize as Bellboy's; surface external ones to the user.
    let caddyfile_path = config_store::caddyfile_path().map_err(StartError::other)?;
    match caddy_supervisor::inspect(state.caddy.current_pid(), &caddyfile_path) {
        CaddySighting::Foreign { bellboy_owned, external } if external.is_empty() => {
            let pids: Vec<u32> = bellboy_owned.iter().map(|p| p.pid).collect();
            caddy_supervisor::kill_pids(&pids).map_err(StartError::other)?;
            // Give the kernel a moment to release the listening sockets before
            // we try to bind them ourselves.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        CaddySighting::Foreign { bellboy_owned, external } => {
            return Err(StartError::ForeignCaddyDetected {
                message: "다른 Caddy 프로세스가 이미 실행 중입니다. 종료한 뒤 다시 시작할 수 있습니다.".into(),
                bellboy_owned,
                external,
            });
        }
        CaddySighting::OursDead => {
            let _ = caddy_process::remove_pid_file();
        }
        _ => {}
    }

    let cfg = config_store::load().map_err(StartError::other)?;
    let caddyfile_text = build_caddyfile(&cfg);
    let path = caddyfile_path;
    std::fs::write(&path, caddyfile_text).map_err(StartError::other)?;

    let domains: Vec<String> = cfg
        .sites
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.domain.clone())
        .collect();
    hosts::sync(&domains).map_err(StartError::other)?;

    state.caddy.start(&path).map_err(StartError::other)?;

    // Caddy may spawn and then die within a few hundred ms (port bind, TLS
    // provisioning, etc.), and its admin API (:2019) usually binds a beat after
    // the process goes live. Poll for readiness instead of a fixed sleep: stop
    // as soon as the process dies (→ failure path below) or the admin socket
    // accepts. A flat 700ms wait used to leave a stale "admin API 응답 없음"
    // warning whenever provisioning ran past that window.
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if !state.caddy.is_running() {
            break;
        }
        if caddy_supervisor::admin_api_reachable() {
            break;
        }
    }

    // Caddy may have (re)generated its local CA. Rebuild the bundle so
    // NODE_EXTRA_CA_CERTS stays up to date for new Node processes.
    if node_env::is_enabled() {
        if let Err(e) = node_env::rebuild_bundle() {
            eprintln!("[bellboy] ca-bundle rebuild failed: {e}");
        }
    }

    if !state.caddy.is_running() {
        let detail = caddy_process::recent_failure_summary().unwrap_or_default();
        let message = if detail.is_empty() {
            "Caddy가 시작 직후 종료되었습니다. 로그: ~/Library/Application Support/bellboy/caddy.log"
                .to_string()
        } else {
            format!("Caddy가 시작 직후 종료되었습니다.\n\n{}", detail)
        };
        return Err(StartError::Other { message });
    }

    Ok(())
}

#[tauri::command]
async fn stop_caddy(state: State<'_, AppState>) -> CmdResult<()> {
    state.caddy.stop().map_err(err)
}

#[tauri::command]
async fn repair_caddy_permissions() -> CmdResult<()> {
    caddy_permissions::repair().map_err(err)
}

#[tauri::command]
fn refresh_health(state: State<'_, AppState>) -> CmdResult<CaddyHealth> {
    let caddyfile = config_store::caddyfile_path().map_err(err)?;
    let sighting = caddy_supervisor::inspect(state.caddy.current_pid(), &caddyfile);
    Ok(CaddyHealth {
        is_running: state.caddy.is_running(),
        admin_api_reachable: caddy_supervisor::admin_api_reachable(),
        sighting,
    })
}

/// Terminates the listed PIDs. The frontend passes the PIDs surfaced via
/// `ForeignCaddyDetected` or `refresh_health` — caller-side confirmation must
/// happen before invoking this.
#[tauri::command]
async fn kill_foreign_caddy(pids: Vec<u32>) -> CmdResult<()> {
    caddy_supervisor::kill_pids(&pids).map_err(err)?;
    // Give the kernel a moment to release the sockets before any follow-up start.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    Ok(())
}

#[tauri::command]
fn get_node_extra_ca_certs() -> bool {
    node_env::is_enabled()
}

#[tauri::command]
async fn set_node_extra_ca_certs(enabled: bool) -> CmdResult<()> {
    if enabled {
        node_env::enable().map_err(err)
    } else {
        node_env::disable().map_err(err)
    }
}

#[tauri::command]
fn get_certificate_trust_status() -> CmdResult<CertificateTrustStatus> {
    system_trust::status().map_err(err)
}

/// Caddy(엔진) / Homebrew 설치 여부 스냅샷. 프론트가 미설치 배너를 그리는 데 사용.
#[tauri::command]
fn get_dependency_status() -> caddy_install::DependencyStatus {
    caddy_install::status()
}

/// `brew install caddy`를 실행하고, 끝나면 갱신된 설치 상태를 반환합니다.
/// 네트워크 다운로드를 동반하는 블로킹 작업이라 별도 스레드에서 돌립니다.
#[tauri::command]
async fn install_caddy() -> CmdResult<caddy_install::DependencyStatus> {
    tauri::async_runtime::spawn_blocking(|| {
        caddy_install::install_caddy()?;
        Ok::<_, anyhow::Error>(caddy_install::status())
    })
    .await
    .map_err(err)?
    .map_err(err)
}

#[tauri::command]
async fn trust_caddy_certificate() -> CmdResult<CertificateTrustStatus> {
    tauri::async_runtime::spawn_blocking(system_trust::trust_caddy_root)
        .await
        .map_err(err)?
        .map_err(err)
}

/// Cleans up Caddy state left over from a previous Bellboy session: stale PID
/// files plus any orphaned caddy process still using *our* Caddyfile.
///
/// We intentionally do not touch external caddies here — only Bellboy-spawned
/// orphans. External processes get surfaced through `refresh_health` and
/// `start_caddy`'s pre-flight, where the user can confirm before we kill them.
fn reconcile_on_boot() {
    let Ok(caddyfile) = config_store::caddyfile_path() else {
        return;
    };
    // `our_pid = None` — we have no in-process child yet at boot.
    if let CaddySighting::Foreign { bellboy_owned, .. } =
        caddy_supervisor::inspect(None, &caddyfile)
    {
        if !bellboy_owned.is_empty() {
            let pids: Vec<u32> = bellboy_owned.iter().map(|p| p.pid).collect();
            let _ = caddy_supervisor::kill_pids(&pids);
        }
    }
    // The pid file points at a process we no longer track; even if it's the
    // one we just killed, the file is meaningless now.
    let _ = caddy_process::remove_pid_file();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    reconcile_on_boot();

    tauri::Builder::default()
        .manage(AppState {
            caddy: CaddyState::new(),
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            generate_caddyfile,
            caddy_status,
            apply_config,
            start_caddy,
            stop_caddy,
            repair_caddy_permissions,
            refresh_health,
            kill_foreign_caddy,
            get_node_extra_ca_certs,
            set_node_extra_ca_certs,
            get_certificate_trust_status,
            trust_caddy_certificate,
            get_dependency_status,
            install_caddy,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

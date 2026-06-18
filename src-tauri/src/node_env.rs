/// Manages `NODE_EXTRA_CA_CERTS` for the user's GUI session so that Node.js
/// server-side code (e.g. Next.js SSR) trusts HTTPS requests to Caddy-managed
/// local domains like `dev.lfind.io.kr`.
///
/// Root cause: Node.js uses its own bundled CA list, not the macOS keychain.
/// Caddy's `tls internal` certs are signed by Caddy's local CA, stored at
/// `~/Library/Application Support/Caddy/pki/authorities/local/root.crt`.
/// This CA is NOT in the macOS system bundle (/etc/ssl/cert.pem), so Node
/// rejects TLS connections to local Caddy domains with
/// `UNABLE_TO_GET_ISSUER_CERT_LOCALLY`.
///
/// Fix: create a combined PEM bundle = system CAs + Caddy local CA, then
/// point `NODE_EXTRA_CA_CERTS` at that bundle. Rebuilt every time Caddy starts
/// (in case Caddy ever regenerates its CA).
///
/// Caveat — Next.js dev: its SSR/render workers are spawned with a sanitized
/// environment that does NOT inherit `NODE_EXTRA_CA_CERTS`, so the bundle alone
/// never reaches them and chat-host calls keep failing. `NODE_OPTIONS`, however,
/// IS propagated to those workers, so the managed ~/.zshenv block also exports
/// `NODE_OPTIONS=--use-system-ca` (Node 22.15+) to make them trust the Caddy
/// root via the macOS keychain. Guarded so it is only added once.
use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::process::Command;

const LABEL: &str = "io.bellboy.node-tls";
const ENV_VAR: &str = "NODE_EXTRA_CA_CERTS";

// macOS system CA bundle (public roots — needed for external HTTPS calls too)
const SYSTEM_CA_PATH: &str = "/etc/ssl/cert.pem";

fn plist_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME not set"))?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", LABEL)))
}

fn caddy_root_cert_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home)
        .join("Library/Application Support/Caddy/pki/authorities/local/root.crt");
    path.exists().then_some(path)
}

fn bundle_path() -> Result<PathBuf> {
    Ok(crate::config_store::app_dir()?.join("ca-bundle.pem"))
}

/// Whether the LaunchAgent plist is installed (= feature is enabled).
pub fn is_enabled() -> bool {
    plist_path().map(|p| p.exists()).unwrap_or(false)
}

/// Rebuilds the combined CA bundle: system CAs + Caddy local root CA.
/// Called on enable AND after each Caddy start so the bundle stays fresh.
/// Returns the bundle path on success.
pub fn rebuild_bundle() -> Result<PathBuf> {
    let out = bundle_path()?;

    let mut content = String::new();

    // 1. macOS system public CA roots
    if let Ok(sys) = std::fs::read_to_string(SYSTEM_CA_PATH) {
        content.push_str(&sys);
    }

    // 2. Caddy local development CA (signs *.test, *.io.kr local domains, etc.)
    if let Some(caddy_root) = caddy_root_cert_path() {
        match std::fs::read_to_string(&caddy_root) {
            Ok(cert) => {
                content.push_str("\n# Caddy local CA (managed by Bellboy)\n");
                content.push_str(&cert);
            }
            Err(e) => {
                // Non-fatal: bundle still usable for public CAs
                eprintln!("[bellboy] warning: could not read Caddy root cert: {e}");
            }
        }
    } else {
        eprintln!("[bellboy] warning: Caddy root cert not found — start Caddy first");
    }

    std::fs::write(&out, content)?;
    Ok(out)
}

/// Writes the LaunchAgent plist and applies the env var to the current session.
/// Also injects into ~/.zshenv so terminal-launched processes (pnpm dev, etc.)
/// pick it up without needing a full logout/login.
pub fn enable() -> Result<()> {
    let bundle = rebuild_bundle()?;
    let bundle_str = bundle
        .to_str()
        .ok_or_else(|| anyhow!("bundle path is not valid UTF-8"))?;

    let plist = plist_path()?;
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&plist, plist_content(bundle_str))?;

    // Load so it takes effect for GUI apps without a logout.
    let plist_str = plist.to_string_lossy();
    let _ = Command::new("launchctl")
        .args(["load", "-w", &plist_str])
        .output();

    // Set immediately in the launchd session env.
    let _ = Command::new("launchctl")
        .args(["setenv", ENV_VAR, bundle_str])
        .output();

    // Also write to ~/.zshenv so terminal-spawned Node processes inherit it.
    let _ = zshenv_add(bundle_str);

    Ok(())
}

/// Removes the LaunchAgent plist, clears the env var, and removes the ~/.zshenv entry.
pub fn disable() -> Result<()> {
    let plist = plist_path()?;
    if plist.exists() {
        let plist_str = plist.to_string_lossy();
        let _ = Command::new("launchctl")
            .args(["unload", "-w", &plist_str])
            .output();
        std::fs::remove_file(&plist)?;
    }

    let _ = Command::new("launchctl")
        .args(["unsetenv", ENV_VAR])
        .output();

    let _ = zshenv_remove();

    Ok(())
}

const ZSHENV_BLOCK_START: &str = "# >>> bellboy node-tls (managed — do not edit)";
const ZSHENV_BLOCK_END: &str = "# <<< bellboy node-tls";
/// Older builds wrote a single comment line + one export instead of a block.
const ZSHENV_LEGACY_MARKER: &str = "# bellboy:node-tls";

fn zshenv_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME not set"))?;
    Ok(PathBuf::from(home).join(".zshenv"))
}

/// Builds the managed ~/.zshenv block: the CA bundle export plus a guarded
/// `NODE_OPTIONS=--use-system-ca` (needed for Next.js workers — see module docs).
fn zshenv_block(bundle_str: &str) -> String {
    let mut b = String::new();
    b.push_str(ZSHENV_BLOCK_START);
    b.push('\n');
    b.push_str("# Node 가 Caddy 로컬 CA 를 신뢰하도록 (로컬 https 도메인 SSR 요청).\n");
    b.push_str("export ");
    b.push_str(ENV_VAR);
    b.push_str("=\"");
    b.push_str(bundle_str);
    b.push_str("\"\n");
    b.push_str("# Next.js dev 의 SSR/렌더 워커는 위 변수를 물려받지 못한다 → system CA(키체인) 사용을 강제한다.\n");
    b.push_str("# (--use-system-ca 는 Node 22.15+ 전용이라, 이미 지정돼 있으면 중복 추가하지 않는다)\n");
    b.push_str("case \":$NODE_OPTIONS:\" in\n");
    b.push_str("  *:--use-system-ca:*) ;;\n");
    b.push_str("  *) export NODE_OPTIONS=\"${NODE_OPTIONS:+$NODE_OPTIONS }--use-system-ca\" ;;\n");
    b.push_str("esac\n");
    b.push_str(ZSHENV_BLOCK_END);
    b
}

/// Removes Bellboy's managed region — both the current START/END block and the
/// legacy single-line `# bellboy:node-tls` marker form — leaving the rest intact.
fn strip_managed(existing: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut inside = false;
    let mut drop_next_export = false;
    for line in existing.lines() {
        let t = line.trim();
        if t == ZSHENV_BLOCK_START {
            inside = true;
            continue;
        }
        if t == ZSHENV_BLOCK_END {
            inside = false;
            continue;
        }
        if inside {
            continue;
        }
        // Legacy form: a lone marker comment followed by one export line.
        if t == ZSHENV_LEGACY_MARKER {
            drop_next_export = true;
            continue;
        }
        if drop_next_export && t.starts_with("export NODE_EXTRA_CA_CERTS=") {
            drop_next_export = false;
            continue;
        }
        drop_next_export = false;
        out.push(line);
    }
    while matches!(out.last(), Some(l) if l.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

fn zshenv_add(bundle_str: &str) -> Result<()> {
    let path = zshenv_path()?;
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut content = strip_managed(&existing);
    if !content.is_empty() {
        content.push_str("\n\n");
    }
    content.push_str(&zshenv_block(bundle_str));
    content.push('\n');
    std::fs::write(&path, content)?;
    Ok(())
}

fn zshenv_remove() -> Result<()> {
    let path = zshenv_path()?;
    let existing = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    let mut content = strip_managed(&existing);
    if !content.is_empty() {
        content.push('\n');
    }
    std::fs::write(&path, content)?;
    Ok(())
}

fn plist_content(bundle_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/launchctl</string>
        <string>setenv</string>
        <string>{env_var}</string>
        <string>{bundle_path}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
        label = LABEL,
        env_var = ENV_VAR,
        bundle_path = bundle_path,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_contains_both_levers() {
        let b = zshenv_block("/tmp/ca.pem");
        assert!(b.starts_with(ZSHENV_BLOCK_START));
        assert!(b.trim_end().ends_with(ZSHENV_BLOCK_END));
        assert!(b.contains("export NODE_EXTRA_CA_CERTS=\"/tmp/ca.pem\""));
        assert!(b.contains("--use-system-ca"));
    }

    #[test]
    fn strip_removes_own_block_and_is_idempotent() {
        let base = ". \"$HOME/.cargo/env\"";
        let once = format!("{base}\n\n{}", zshenv_block("/tmp/ca.pem"));
        assert_eq!(strip_managed(&once), base);
        // A second managed block (e.g. accidental double-add) is fully removed too.
        let twice = format!("{once}\n\n{}", zshenv_block("/tmp/ca.pem"));
        assert_eq!(strip_managed(&twice), base);
    }

    #[test]
    fn strip_migrates_legacy_single_line_marker() {
        let legacy = ". \"$HOME/.cargo/env\"\n\n# bellboy:node-tls\nexport NODE_EXTRA_CA_CERTS=\"/old/perch/ca-bundle.pem\"\n";
        let stripped = strip_managed(legacy);
        assert!(!stripped.contains("perch"));
        assert!(!stripped.contains("bellboy:node-tls"));
        assert!(stripped.contains(".cargo/env"));
    }
}

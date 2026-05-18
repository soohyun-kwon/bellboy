//! 관리자 권한 자동 캐싱.
//!
//! # 흐름
//! 1. 권한이 필요한 작업 → `sudo -n /usr/local/bin/perch-helper <cmd>` 먼저 시도
//! 2. 실패(첫 실행) → osascript 한 번으로 **실제 작업 + 래퍼 스크립트 + sudoers + pam_tid** 설치
//! 3. 이후 재실행 → `sudo -n` 성공, 프롬프트 없음
//!
//! # 왜 래퍼 스크립트?
//! macOS의 visudo는 sudoers 규칙 경로에 공백이나 특수문자를 허용하지 않습니다.
//! (`Application Support` 경로에서 `\ `·`?` 모두 문법 오류)
//! 래퍼 스크립트를 `/usr/local/bin/perch-helper`(공백 없음)에 두면
//! sudoers 규칙이 단순해지고 검증도 통과합니다.
//!
//! # Touch ID
//! `/etc/pam.d/sudo_local`에 `auth sufficient pam_tid.so` 추가 (macOS 13.3+).
//! 시스템 업데이트에 덮어써지지 않으며, 터미널의 sudo 명령에도 지문이 적용됩니다.

use anyhow::{anyhow, Context, Result};
use std::process::Command;

pub const HOSTS_STAGING_PATH: &str = "/tmp/perch-hosts-staging";
pub const HELPER_PATH: &str = "/usr/local/bin/perch-helper";
const SUDOERS_PATH: &str = "/etc/sudoers.d/perch";
const PAM_SUDO_LOCAL_PATH: &str = "/etc/pam.d/sudo_local";

pub fn is_installed() -> bool {
    std::path::Path::new(HELPER_PATH).exists()
        && std::path::Path::new(SUDOERS_PATH).exists()
}

/// 권한이 필요한 작업과 전체 설정을 **하나의 osascript 프롬프트**로 처리합니다.
///
/// `operation`: `"sync-hosts"` 또는 `"repair-caddy"`.
/// 설치 완료 후 해당 operation을 바로 실행합니다.
pub fn install_and_run(operation: Operation) -> Result<()> {
    let (user, home) = user_and_home()?;
    let caddy_dir = format!("{}/Library/Application Support/Caddy", home);

    // 래퍼 스크립트 내용을 생성합니다. user/caddy_dir을 하드코딩해 런타임 변수 의존을 없앱니다.
    let helper_content = build_helper_script(&user, &caddy_dir)?;
    let sudoers_content = build_sudoers_content(&user)?;
    let pam_content = "# Managed by Perch\nauth       sufficient     pam_tid.so\n";

    // 임시 파일에 먼저 써둡니다 (user 권한으로 가능).
    let pid = std::process::id();
    let helper_tmp = format!("/tmp/perch-helper-{}.tmp", pid);
    let sudoers_tmp = format!("/tmp/perch-sudoers-{}.tmp", pid);
    let pam_tmp = format!("/tmp/perch-pam-{}.tmp", pid);

    std::fs::write(&helper_tmp, &helper_content)
        .with_context(|| format!("stage helper at {}", helper_tmp))?;
    std::fs::write(&sudoers_tmp, &sudoers_content)
        .with_context(|| format!("stage sudoers at {}", sudoers_tmp))?;
    std::fs::write(&pam_tmp, pam_content)
        .with_context(|| format!("stage pam at {}", pam_tmp))?;

    validate_sudoers_content(&sudoers_content)?;

    // osascript 샌드박스에서 `/usr/bin/install`은 임시 파일 생성이 막혀 실패합니다.
    // cp + chmod + chown 으로 동일한 결과를 얻습니다.
    let mut steps = vec![
        "/bin/mkdir -p /usr/local/bin".to_string(),
        install_file(&helper_tmp, HELPER_PATH, "0755"),
        install_file(&sudoers_tmp, SUDOERS_PATH, "0440"),
    ];
    if !pam_sudo_local_has_tid() {
        steps.push(install_file(&pam_tmp, PAM_SUDO_LOCAL_PATH, "0644"));
    }
    // 마지막으로 실제 작업을 실행합니다 (설치 성공 확인 겸).
    steps.push(format!("{} {}", HELPER_PATH, operation.as_str()));

    let osa = format!(
        "do shell script \"{}\" with administrator privileges",
        steps.join(" && ")
    );
    let output = Command::new("osascript")
        .args(["-e", &osa])
        .output()
        .context("spawn osascript")?;

    let _ = std::fs::remove_file(&helper_tmp);
    let _ = std::fs::remove_file(&sudoers_tmp);
    let _ = std::fs::remove_file(&pam_tmp);

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        if err.contains("User canceled") || err.contains("-128") {
            return Err(anyhow!("사용자가 관리자 권한 요청을 취소했습니다"));
        }
        return Err(anyhow!("osascript failed: {}", err));
    }
    Ok(())
}

/// 설치된 래퍼 스크립트를 통해 `sudo -n`으로 실행합니다. 프롬프트 없음.
pub fn run(operation: Operation) -> Result<()> {
    let output = Command::new("/usr/bin/sudo")
        .args(["-n", HELPER_PATH, operation.as_str()])
        .output()
        .context("spawn sudo")?;
    if !output.status.success() {
        return Err(anyhow!(
            "sudo helper failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

/// 래퍼 스크립트·sudoers·pam_tid를 제거합니다.
pub fn uninstall() -> Result<()> {
    let mut script = format!(
        "/bin/rm -f '{}' '{}'",
        HELPER_PATH, SUDOERS_PATH
    );
    if pam_sudo_local_has_tid() {
        script.push_str(&format!(" && /bin/rm -f '{}'", PAM_SUDO_LOCAL_PATH));
    }
    let osa = format!(
        "do shell script \"{}\" with administrator privileges",
        script
    );
    let output = Command::new("osascript")
        .args(["-e", &osa])
        .output()
        .context("spawn osascript")?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        if err.contains("User canceled") || err.contains("-128") {
            return Err(anyhow!("사용자가 관리자 권한 요청을 취소했습니다"));
        }
        return Err(anyhow!("제거 실패: {}", err));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub enum Operation {
    SyncHosts,
    RepairCaddy,
}

impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Operation::SyncHosts => "sync-hosts",
            Operation::RepairCaddy => "repair-caddy",
        }
    }
}

// ─── 내부 헬퍼 ──────────────────────────────────────────────────────────────

/// user와 caddy_dir을 하드코딩한 래퍼 스크립트를 생성합니다.
/// 런타임에 $HOME이나 $SUDO_USER에 의존하지 않아 안전합니다.
fn build_helper_script(user: &str, caddy_dir: &str) -> Result<String> {
    // 셸 인젝션 방지: user는 알파벳/숫자만, caddy_dir은 싱글쿼트 없음 확인
    if !is_safe_name(user) {
        return Err(anyhow!("USER에 허용되지 않는 문자: {}", user));
    }
    if caddy_dir.contains('\'') || caddy_dir.contains('\\') {
        return Err(anyhow!("Caddy 경로에 허용되지 않는 문자가 있습니다"));
    }

    Ok(format!(
        "#!/bin/bash\n\
         set -euo pipefail\n\
         case \"${{1:-}}\" in\n\
           sync-hosts)\n\
             exec /bin/cp '{staging}' /etc/hosts\n\
             ;;\n\
           repair-caddy)\n\
             exec /usr/sbin/chown -R '{user}:staff' '{caddy}'\n\
             ;;\n\
           *)\n\
             echo \"perch-helper: unknown command: ${{1:-}}\" >&2\n\
             exit 1\n\
             ;;\n\
         esac\n",
        staging = HOSTS_STAGING_PATH,
        user = user,
        caddy = caddy_dir,
    ))
}

fn build_sudoers_content(user: &str) -> Result<String> {
    if !is_safe_name(user) {
        return Err(anyhow!("USER에 허용되지 않는 문자: {}", user));
    }
    Ok(format!(
        "# Managed by Perch — do not edit manually\n\
         {user} ALL=(root) NOPASSWD: {helper} sync-hosts\n\
         {user} ALL=(root) NOPASSWD: {helper} repair-caddy\n",
        user = user,
        helper = HELPER_PATH,
    ))
}

fn validate_sudoers_content(content: &str) -> Result<()> {
    let tmp = format!("/tmp/perch-sudoers-validate-{}.tmp", std::process::id());
    std::fs::write(&tmp, content).context("write validation temp")?;
    let out = Command::new("/usr/sbin/visudo")
        .args(["-c", "-f", &tmp])
        .output();
    let _ = std::fs::remove_file(&tmp);
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(anyhow!(
            "sudoers 검증 실패: {}",
            String::from_utf8_lossy(&o.stderr)
        )),
        Err(e) => Err(anyhow!("visudo 실행 실패: {}", e)),
    }
}

/// `cp src dst && chmod mode dst && chown root:wheel dst` 명령 문자열을 반환합니다.
/// `/usr/bin/install` 대신 사용 — install은 osascript 샌드박스에서 임시 파일 생성이
/// 막혀 "Operation not permitted" 오류를 냅니다.
fn install_file(src: &str, dst: &str, mode: &str) -> String {
    format!(
        "/bin/cp '{src}' '{dst}' && /bin/chmod {mode} '{dst}' && /usr/sbin/chown root:wheel '{dst}'"
    )
}

fn pam_sudo_local_has_tid() -> bool {
    std::fs::read_to_string(PAM_SUDO_LOCAL_PATH)
        .map(|c| c.contains("pam_tid.so"))
        .unwrap_or(false)
}

fn user_and_home() -> Result<(String, String)> {
    let user = std::env::var("USER").context("USER not set")?;
    if !is_safe_name(&user) {
        return Err(anyhow!("USER에 허용되지 않는 문자: {}", user));
    }
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok((user, home))
}

fn is_safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_script_contains_expected_commands() {
        let script = build_helper_script("alice", "/Users/alice/Library/Application Support/Caddy").unwrap();
        assert!(script.contains("sync-hosts"));
        assert!(script.contains("repair-caddy"));
        assert!(script.contains("/bin/cp '/tmp/perch-hosts-staging' /etc/hosts"));
        assert!(script.contains("/usr/sbin/chown -R 'alice:staff'"));
        assert!(script.contains("Application Support/Caddy"));
    }

    #[test]
    fn sudoers_content_has_no_spaces_in_paths() {
        let content = build_sudoers_content("alice").unwrap();
        // helper 경로에 공백 없음
        for line in content.lines().filter(|l| l.contains("NOPASSWD")) {
            let cmd_part = line.split("NOPASSWD:").nth(1).unwrap_or("");
            // /usr/local/bin/perch-helper 부분만 검사
            let helper = cmd_part.trim().split_whitespace().next().unwrap_or("");
            assert!(!helper.contains(' '), "helper path has space: {}", helper);
        }
    }

    #[test]
    fn safe_name_accepts_typical() {
        assert!(is_safe_name("alice"));
        assert!(is_safe_name("john.doe"));
        assert!(is_safe_name("user-1"));
    }

    #[test]
    fn safe_name_rejects_shell_metachars() {
        assert!(!is_safe_name(""));
        assert!(!is_safe_name("user space"));
        assert!(!is_safe_name("a;b"));
        assert!(!is_safe_name("a$b"));
    }
}

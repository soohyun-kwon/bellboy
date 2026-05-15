//! 관리자 권한 자동 캐싱.
//!
//! # 흐름
//! 1. 권한이 필요한 작업 시도 → `sudo -n` 먼저 시도
//! 2. 실패(첫 실행) → osascript로 **실제 작업 + sudoers 설치 + Touch ID(pam_tid) 설치**를
//!    하나의 프롬프트로 처리
//! 3. 이후 재실행 → `sudo -n` 성공, 프롬프트 없음
//!
//! # Touch ID
//! `/etc/pam.d/sudo_local`에 `auth sufficient pam_tid.so`를 추가합니다.
//! macOS 13.3+에서 `sudo_local`은 시스템 업데이트에 덮어써지지 않습니다.
//! sudo 자체가 Touch ID를 지원하게 되므로, 다른 터미널 sudo 명령에도 지문이 적용됩니다.
//!
//! # 범위
//! sudoers 규칙은 Perch가 실제로 필요한 두 명령만 허용합니다:
//! - `/bin/cp /tmp/perch-hosts-staging /etc/hosts`
//! - `/usr/sbin/chown -R <user>:staff <caddy-dir>`

use anyhow::{anyhow, Context, Result};
use std::process::Command;

/// hosts 스테이징 고정 경로. 공백 없는 경로여야 sudoers 규칙이 정확히 매칭됩니다.
pub const HOSTS_STAGING_PATH: &str = "/tmp/perch-hosts-staging";
const SUDOERS_PATH: &str = "/etc/sudoers.d/perch";
const PAM_SUDO_LOCAL_PATH: &str = "/etc/pam.d/sudo_local";

pub fn is_installed() -> bool {
    std::path::Path::new(SUDOERS_PATH).exists()
}

/// 권한이 필요한 작업과 sudoers/pam 설치를 **하나의 osascript 프롬프트**로 처리합니다.
///
/// `main_shell_cmd`: 실제로 실행해야 할 셸 명령 (예: `/bin/cp '/tmp/...' '/etc/hosts'`).
/// 이 명령 뒤에 sudoers + pam_tid 설치 명령을 `&&`로 이어 붙입니다.
pub fn install_with_command(main_shell_cmd: &str) -> Result<()> {
    let (user, caddy_dir) = user_and_caddy_dir()?;
    let caddy_dir_pattern = caddy_dir.replace(' ', "?");

    let sudoers_content = format!(
        "# Managed by Perch — do not edit manually\n\
         # Allows Perch to update /etc/hosts and repair Caddy permissions\n\
         # without prompting for a password on every restart.\n\
         {user} ALL=(root) NOPASSWD: /bin/cp {staging} /etc/hosts\n\
         {user} ALL=(root) NOPASSWD: /usr/sbin/chown -R {user}:staff {caddy}\n",
        user = user,
        staging = HOSTS_STAGING_PATH,
        caddy = caddy_dir_pattern,
    );
    validate_sudoers_content(&sudoers_content)?;

    // osascript 실행 전에 임시 파일들을 먼저 써둡니다 (user 권한으로 가능).
    let sudoers_tmp = format!("/tmp/perch-sudoers-{}.tmp", std::process::id());
    let pam_tmp = format!("/tmp/perch-pam-{}.tmp", std::process::id());

    std::fs::write(&sudoers_tmp, &sudoers_content)
        .with_context(|| format!("stage sudoers at {}", sudoers_tmp))?;

    let pam_content = "# Managed by Perch\nauth       sufficient     pam_tid.so\n";
    std::fs::write(&pam_tmp, pam_content)
        .with_context(|| format!("stage pam at {}", pam_tmp))?;

    // 세 작업을 하나의 셸 명령으로 이어 붙입니다:
    //   1. 실제 작업 (hosts cp 또는 chown)
    //   2. sudoers 드롭인 설치
    //   3. pam_tid.so 설치 (없는 경우에만)
    let mut steps = vec![
        main_shell_cmd.to_string(),
        format!(
            "/usr/bin/install -m 0440 -o root -g wheel '{sudoers_tmp}' '{SUDOERS_PATH}'"
        ),
    ];
    if !pam_sudo_local_has_tid() {
        steps.push(format!(
            "/usr/bin/install -m 0644 -o root -g wheel '{pam_tmp}' '{PAM_SUDO_LOCAL_PATH}'"
        ));
    }

    let combined = steps.join(" && ");
    let osa = format!(
        "do shell script \"{}\" with administrator privileges",
        combined
    );

    let output = Command::new("osascript")
        .args(["-e", &osa])
        .output()
        .context("spawn osascript")?;

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

/// sudoers 드롭인과 pam_tid 설정을 제거합니다.
pub fn uninstall() -> Result<()> {
    let mut script = format!("/bin/rm -f '{SUDOERS_PATH}'");
    if pam_sudo_local_has_tid() {
        script.push_str(&format!(" && /bin/rm -f '{PAM_SUDO_LOCAL_PATH}'"));
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

// ─── 내부 헬퍼 ──────────────────────────────────────────────────────────────

fn user_and_caddy_dir() -> Result<(String, String)> {
    let user = std::env::var("USER").context("USER not set")?;
    if !is_safe_name(&user) {
        return Err(anyhow!("USER에 허용되지 않는 문자가 있습니다: {}", user));
    }
    let home = std::env::var("HOME").context("HOME not set")?;
    let caddy_dir = format!("{}/Library/Application Support/Caddy", home);
    Ok((user, caddy_dir))
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

fn pam_sudo_local_has_tid() -> bool {
    std::fs::read_to_string(PAM_SUDO_LOCAL_PATH)
        .map(|c| c.contains("pam_tid.so"))
        .unwrap_or(false)
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
    fn safe_name_accepts_typical() {
        assert!(is_safe_name("gwonsuhyeon"));
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

    #[test]
    fn caddy_dir_spaces_become_glob_wildcard() {
        let caddy = "/Users/me/Library/Application Support/Caddy";
        let pattern = caddy.replace(' ', "?");
        assert_eq!(pattern, "/Users/me/Library/Application?Support/Caddy");
        assert!(!pattern.contains(' '));
        assert!(!pattern.contains('\\'));
    }
}

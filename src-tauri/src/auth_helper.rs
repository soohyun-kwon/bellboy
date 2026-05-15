//! One-time privileged-helper setup.
//!
//! # 문제
//! Perch는 `/etc/hosts` 편집과 Caddy 데이터 디렉토리 소유권 복구에 매번 osascript
//! admin 프롬프트를 띄웁니다. 앱을 껐다 켤 때마다 비밀번호를 입력해야 합니다.
//!
//! # 해결
//! `/etc/sudoers.d/perch` 드롭인 파일을 **한 번** 설치합니다. 이후 Perch는 해당
//! 명령에 한해 `sudo -n`으로 비밀번호 없이 실행합니다. osascript 프롬프트는 오직
//! 최초 설치 시에만 표시됩니다.
//!
//! Touch ID 옵션을 켜면 `/etc/pam.d/sudo_local`에 `auth sufficient pam_tid.so`를
//! 추가해 이후 sudo 인증 시 지문을 사용할 수 있게 됩니다. (macOS 13.3+ 에서
//! `sudo_local`은 시스템 업데이트에 덮어써지지 않는 안전한 파일입니다.)
//!
//! # 범위
//! sudoers 규칙은 Perch가 필요한 명령 두 개만 허용합니다:
//! - `/bin/cp <staging> /etc/hosts`   — hosts 동기화
//! - `/usr/sbin/chown -R <user>:staff <caddy-dir>` — 소유권 복구

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::process::Command;

/// Hosts 파일 스테이징 경로. 스페이스가 없는 고정 경로를 써야 sudoers 규칙이
/// 인수를 정확히 매칭할 수 있습니다.
pub const HOSTS_STAGING_PATH: &str = "/tmp/perch-hosts-staging";
const SUDOERS_PATH: &str = "/etc/sudoers.d/perch";
const PAM_SUDO_LOCAL_PATH: &str = "/etc/pam.d/sudo_local";

#[derive(Debug, Clone, Serialize)]
pub struct AuthHelperStatus {
    pub is_installed: bool,
    pub touchid_enabled: bool,
}

pub fn status() -> AuthHelperStatus {
    AuthHelperStatus {
        is_installed: std::path::Path::new(SUDOERS_PATH).exists(),
        touchid_enabled: pam_sudo_local_has_tid(),
    }
}

/// sudoers 드롭인을 설치합니다. `enable_touchid`가 true이면 `/etc/pam.d/sudo_local`에
/// Touch ID 줄도 함께 추가합니다. osascript 프롬프트가 한 번 뜹니다.
pub fn install(enable_touchid: bool) -> Result<()> {
    let (user, caddy_dir) = user_and_caddy_dir()?;

    // sudoers에서 스페이스는 `\ ` 로 이스케이프합니다.
    let caddy_dir_escaped = caddy_dir.replace(' ', r"\ ");

    let sudoers_content = format!(
        "# Managed by Perch — do not edit manually\n\
         # Allows Perch to update /etc/hosts and repair Caddy permissions\n\
         # without prompting for a password on every restart.\n\
         {user} ALL=(root) NOPASSWD: /bin/cp {staging} /etc/hosts\n\
         {user} ALL=(root) NOPASSWD: /usr/sbin/chown -R {user}:staff {caddy}\n",
        user = user,
        staging = HOSTS_STAGING_PATH,
        caddy = caddy_dir_escaped,
    );

    validate_sudoers_content(&sudoers_content)?;

    // 임시 파일에 먼저 써두고 osascript로 제자리에 설치합니다.
    let tmp = format!("/tmp/perch-sudoers-{}.tmp", std::process::id());
    std::fs::write(&tmp, &sudoers_content)
        .with_context(|| format!("stage sudoers at {}", tmp))?;

    let mut script = format!(
        "/usr/bin/install -m 0440 -o root -g wheel '{tmp}' '{dst}'",
        tmp = tmp,
        dst = SUDOERS_PATH
    );

    if enable_touchid {
        // sudo_local은 시스템 업데이트에도 살아남는 macOS 13.3+ 권장 방식입니다.
        // 이미 pam_tid 줄이 있으면 중복 추가하지 않습니다.
        if !pam_sudo_local_has_tid() {
            let pam_content =
                "# Managed by Perch\nauth       sufficient     pam_tid.so\n";
            let pam_tmp = format!("/tmp/perch-pam-{}.tmp", std::process::id());
            std::fs::write(&pam_tmp, pam_content)
                .with_context(|| format!("stage pam at {}", pam_tmp))?;
            script.push_str(&format!(
                " && /usr/bin/install -m 0644 -o root -g wheel '{pam_tmp}' '{dst}'",
                pam_tmp = pam_tmp,
                dst = PAM_SUDO_LOCAL_PATH
            ));
        }
    }

    let osa = format!(
        "do shell script \"{}\" with administrator privileges",
        script
    );
    let output = Command::new("osascript")
        .args(["-e", &osa])
        .output()
        .context("spawn osascript")?;

    // 임시 파일 정리 (실패해도 계속)
    let _ = std::fs::remove_file(&tmp);

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        if err.contains("User canceled") || err.contains("-128") {
            return Err(anyhow!("사용자가 관리자 권한 요청을 취소했습니다"));
        }
        return Err(anyhow!("설치 실패: {}", err));
    }

    Ok(())
}

/// sudoers 드롭인을 제거합니다. Touch ID 설정(`/etc/pam.d/sudo_local`)도 함께
/// 제거할지 `remove_touchid`로 선택합니다.
pub fn uninstall(remove_touchid: bool) -> Result<()> {
    let mut script = format!("/bin/rm -f '{}'", SUDOERS_PATH);

    if remove_touchid && pam_sudo_local_has_tid() {
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

// ─── 내부 헬퍼 ───────────────────────────────────────────────────────────────

fn user_and_caddy_dir() -> Result<(String, String)> {
    let user = std::env::var("USER").context("USER not set")?;
    if !is_safe_name(&user) {
        return Err(anyhow!("USER에 허용되지 않는 문자가 있습니다: {}", user));
    }
    let home = std::env::var("HOME").context("HOME not set")?;
    let caddy_dir = format!("{}/Library/Application Support/Caddy", home);
    Ok((user, caddy_dir))
}

/// `visudo -c -f`로 생성한 sudoers 내용의 문법을 검증합니다.
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
    fn caddy_dir_spaces_are_escaped() {
        let caddy = "/Users/me/Library/Application Support/Caddy";
        let escaped = caddy.replace(' ', r"\ ");
        assert_eq!(
            escaped,
            r"/Users/me/Library/Application\ Support/Caddy"
        );
        assert!(!escaped.contains("  "));
    }
}

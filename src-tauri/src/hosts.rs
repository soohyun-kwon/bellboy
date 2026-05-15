//! /etc/hosts editor.
//!
//! Strategy: we keep a clearly-marked block at the bottom of /etc/hosts that
//! Perch owns. The rest of the file is untouched. Writing requires sudo, so we
//! stage a temp file and call `osascript` to `cp` it in place using the system
//! admin-privilege prompt. Future: move to an SMAppService helper so we don't
//! prompt on every write.

use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::Command;

const HOSTS_PATH: &str = "/etc/hosts";
const MARKER_START: &str = "# >>> perch managed (do not edit between markers)";
const MARKER_END: &str = "# <<< perch managed";

/// Sync /etc/hosts so that exactly `domains` are mapped to 127.0.0.1 inside the
/// Perch-managed block. Returns without touching sudo if no changes are needed.
pub fn sync(domains: &[String]) -> Result<()> {
    let current =
        std::fs::read_to_string(HOSTS_PATH).with_context(|| format!("read {}", HOSTS_PATH))?;
    let next = merge_block(&current, domains);
    if next == current {
        return Ok(());
    }
    write_with_privilege(&next)
}

fn merge_block(current: &str, domains: &[String]) -> String {
    let mut result = String::new();
    let mut inside = false;
    for line in current.lines() {
        let trimmed = line.trim();
        if trimmed == MARKER_START {
            inside = true;
            continue;
        }
        if trimmed == MARKER_END {
            inside = false;
            continue;
        }
        if inside {
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }

    // Collapse trailing blanks.
    while result.ends_with("\n\n") {
        result.pop();
    }

    if !domains.is_empty() {
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        result.push('\n');
        result.push_str(MARKER_START);
        result.push('\n');
        for d in domains {
            result.push_str(&format!("127.0.0.1\t{}\n", d));
            result.push_str(&format!("::1\t{}\n", d));
        }
        result.push_str(MARKER_END);
        result.push('\n');
    }

    result
}

fn write_with_privilege(content: &str) -> Result<()> {
    // auth_helper가 설치된 경우 고정 경로에 스테이징 후 `sudo -n`으로 실행합니다.
    // 설치되지 않은 경우 기존 osascript 프롬프트로 fallback합니다.
    let staging = std::path::Path::new(crate::auth_helper::HOSTS_STAGING_PATH);
    std::fs::write(staging, content)
        .with_context(|| format!("stage hosts at {}", staging.display()))?;

    let result = if crate::auth_helper::status().is_installed {
        run_sudo_cp(staging)
    } else {
        run_osascript_cp(staging)
    };

    let _ = std::fs::remove_file(staging);
    result
}

/// `sudo -n /bin/cp`로 비밀번호 없이 실행합니다. sudoers 규칙이 없으면 즉시 실패해
/// 호출자가 osascript fallback을 시도합니다.
fn run_sudo_cp(staging: &Path) -> Result<()> {
    let staging_str = staging
        .to_str()
        .ok_or_else(|| anyhow!("staging path not valid UTF-8"))?;
    let output = Command::new("/usr/bin/sudo")
        .args(["-n", "/bin/cp", staging_str, HOSTS_PATH])
        .output()
        .context("spawn sudo")?;
    if !output.status.success() {
        return Err(anyhow!(
            "sudo cp failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

fn run_osascript_cp(src: &Path) -> Result<()> {
    let src_str = src
        .to_str()
        .ok_or_else(|| anyhow!("temp path not valid UTF-8"))?;

    if src_str.contains('"') || src_str.contains('\\') {
        return Err(anyhow!("unexpected characters in temp path"));
    }

    let script = format!(
        "do shell script \"/bin/cp '{}' '{}'\" with administrator privileges",
        src_str, HOSTS_PATH
    );

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("spawn osascript")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        if err.contains("User canceled") || err.contains("-128") {
            return Err(anyhow!("사용자가 관리자 권한 요청을 취소했습니다"));
        }
        return Err(anyhow!("osascript failed: {}", err));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_adds_block_when_absent() {
        let current = "127.0.0.1\tlocalhost\n::1\tlocalhost\n";
        let domains = vec!["myapp.test".to_string(), "other.test".to_string()];
        let out = merge_block(current, &domains);
        assert!(out.contains("127.0.0.1\tlocalhost"));
        assert!(out.contains(MARKER_START));
        assert!(out.contains("127.0.0.1\tmyapp.test"));
        assert!(out.contains("127.0.0.1\tother.test"));
        assert!(out.contains(MARKER_END));
    }

    #[test]
    fn merge_replaces_existing_block() {
        let current = format!(
            "127.0.0.1\tlocalhost\n\n{}\n127.0.0.1\told.test\n{}\n",
            MARKER_START, MARKER_END
        );
        let domains = vec!["new.test".to_string()];
        let out = merge_block(&current, &domains);
        assert!(!out.contains("old.test"));
        assert!(out.contains("new.test"));
    }

    #[test]
    fn merge_removes_block_when_domains_empty() {
        let current = format!(
            "127.0.0.1\tlocalhost\n{}\n127.0.0.1\tgone.test\n{}\n",
            MARKER_START, MARKER_END
        );
        let out = merge_block(&current, &[]);
        assert!(!out.contains(MARKER_START));
        assert!(!out.contains("gone.test"));
        assert!(out.contains("127.0.0.1\tlocalhost"));
    }

    #[test]
    fn merge_idempotent() {
        let current = "127.0.0.1\tlocalhost\n";
        let domains = vec!["a.test".to_string()];
        let once = merge_block(current, &domains);
        let twice = merge_block(&once, &domains);
        assert_eq!(once, twice);
    }
}

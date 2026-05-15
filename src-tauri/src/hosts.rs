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
    let staging = std::path::Path::new(crate::auth_helper::HOSTS_STAGING_PATH);
    std::fs::write(staging, content)
        .with_context(|| format!("stage hosts at {}", staging.display()))?;

    // sudoers가 설치돼 있으면 프롬프트 없이 실행합니다.
    // 아직 설치되지 않은 첫 실행이면, 실제 작업 + sudoers + pam_tid를
    // 하나의 osascript 프롬프트로 처리합니다.
    let result = if crate::auth_helper::is_installed() {
        run_sudo_cp(staging)
    } else {
        let cmd = format!(
            "/bin/cp '{}' '{}'",
            crate::auth_helper::HOSTS_STAGING_PATH,
            HOSTS_PATH
        );
        crate::auth_helper::install_with_command(&cmd)
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

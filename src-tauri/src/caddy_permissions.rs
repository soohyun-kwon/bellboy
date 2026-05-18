//! Pre-flight permission check + in-app repair for Caddy's data directory.
//!
//! Why this exists: Caddy keeps its internal CA and autocert state under
//! `~/Library/Application Support/Caddy`. If the user (or a prior tool) ever
//! ran `sudo caddy ...`, that directory ends up root-owned and a subsequent
//! user-mode run fails with `permission denied` on the root cert.
//!
//! Rather than making the user open a terminal and type `chown`, we detect
//! the condition up-front and offer to repair it via `osascript` — same
//! pattern `hosts.rs` uses to get one admin-prompt and be done with it.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

const CADDY_DIR: &str = "Library/Application Support/Caddy";
const PROBE_FILENAME: &str = ".perch-write-probe";
const ROOT_CRT_REL: &str = "pki/authorities/local/root.crt";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionIssue {
    /// A file inside the PKI tree exists but we can't read it. Almost always
    /// means it's owned by root from a previous `sudo caddy` invocation.
    PkiNotReadable { path: PathBuf },
    /// The data directory itself can't accept writes.
    DirNotWritable { path: PathBuf },
}

impl PermissionIssue {
    pub fn message(&self) -> String {
        match self {
            PermissionIssue::PkiNotReadable { .. } => {
                "Caddy 내부 인증서 파일을 읽을 수 없습니다. 이전에 sudo로 실행된 흔적이라 소유권 복구가 필요합니다.".into()
            }
            PermissionIssue::DirNotWritable { .. } => {
                "Caddy 데이터 폴더에 쓰기 권한이 없습니다. 소유권 복구가 필요합니다.".into()
            }
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            PermissionIssue::PkiNotReadable { path } => path,
            PermissionIssue::DirNotWritable { path } => path,
        }
    }
}

fn caddy_data_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(CADDY_DIR))
}

/// Returns `Ok(())` if Caddy can safely read/write its data dir.
/// Returns `Err(PermissionIssue)` for the specific repairable cases.
/// Unexpected IO failures bubble up as `Ok(())` — we'd rather let Caddy surface
/// them than block startup on a false positive.
pub fn check() -> Result<(), PermissionIssue> {
    let Ok(dir) = caddy_data_dir() else {
        return Ok(());
    };
    if !dir.exists() {
        // Caddy will create it on first run.
        return Ok(());
    }

    let root_crt = dir.join(ROOT_CRT_REL);
    if root_crt.exists() && std::fs::File::open(&root_crt).is_err() {
        return Err(PermissionIssue::PkiNotReadable { path: root_crt });
    }

    let probe = dir.join(PROBE_FILENAME);
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(_) => Err(PermissionIssue::DirNotWritable { path: dir }),
    }
}

/// Recursively chowns the Caddy data dir back to the current user.
/// sudoers가 설치된 경우 프롬프트 없이, 아니면 첫 실행 osascript와 함께 설치합니다.
pub fn repair() -> Result<()> {
    use crate::auth_helper::{Operation, is_installed, install_and_run, run};

    let dir = caddy_data_dir()?;
    if !dir.exists() {
        return Ok(());
    }

    if is_installed() {
        run(Operation::RepairCaddy)
    } else {
        install_and_run(Operation::RepairCaddy)
    }
}

fn is_safe_user(user: &str) -> bool {
    !user.is_empty()
        && user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_user_accepts_typical_names() {
        assert!(is_safe_user("gwonsuhyeon"));
        assert!(is_safe_user("soo-hyun"));
        assert!(is_safe_user("user.name"));
        assert!(is_safe_user("user_1"));
    }

    #[test]
    fn safe_user_rejects_shell_metachars() {
        assert!(!is_safe_user(""));
        assert!(!is_safe_user("user; rm -rf /"));
        assert!(!is_safe_user("user'"));
        assert!(!is_safe_user("user space"));
        assert!(!is_safe_user("user$HOME"));
    }

    #[test]
    fn issue_message_and_path_accessors() {
        let p = PathBuf::from("/tmp/x");
        let issue = PermissionIssue::PkiNotReadable { path: p.clone() };
        assert_eq!(issue.path(), p.as_path());
        assert!(!issue.message().is_empty());

        let issue = PermissionIssue::DirNotWritable { path: p.clone() };
        assert_eq!(issue.path(), p.as_path());
        assert!(!issue.message().is_empty());
    }
}

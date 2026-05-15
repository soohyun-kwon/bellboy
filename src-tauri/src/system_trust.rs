//! macOS system trust integration for Caddy's local CA.
//!
//! Caddy's `tls internal` issues certificates from a local root CA. Browsers can
//! trust it once the root is added to the System Keychain; without that, Node and
//! other clients report issuer errors.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;

const CADDY_DIR: &str = "Library/Application Support/Caddy";
const ROOT_CRT_REL: &str = "pki/authorities/local/root.crt";
const SYSTEM_KEYCHAIN: &str = "/Library/Keychains/System.keychain";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CertificateTrustState {
    Trusted,
    Untrusted,
    RootMissing,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CertificateTrustStatus {
    pub state: CertificateTrustState,
    pub root_path: Option<String>,
    pub message: String,
    pub node_hint: String,
}

pub fn status() -> Result<CertificateTrustStatus> {
    status_for_root_path(root_crt_path()?)
}

pub fn trust_caddy_root() -> Result<CertificateTrustStatus> {
    let root = root_crt_path()?;
    if !root.exists() {
        return Ok(root_missing_status(root));
    }

    run_add_trusted_cert(&root)?;
    status_for_root_path(root)
}

fn status_for_root_path(root: PathBuf) -> Result<CertificateTrustStatus> {
    if !root.exists() {
        return Ok(root_missing_status(root));
    }

    let root_path = Some(root.display().to_string());
    if is_trusted_for_ssl(&root)? {
        return Ok(CertificateTrustStatus {
            state: CertificateTrustState::Trusted,
            root_path,
            message: "Caddy 로컬 CA가 macOS 시스템 신뢰 저장소에 등록되어 있습니다.".into(),
            node_hint: node_system_ca_hint(),
        });
    }

    Ok(CertificateTrustStatus {
        state: CertificateTrustState::Untrusted,
        root_path,
        message: "Caddy 로컬 CA가 아직 macOS 시스템 신뢰 저장소에 등록되어 있지 않습니다.".into(),
        node_hint: node_system_ca_hint(),
    })
}

fn root_missing_status(root: PathBuf) -> CertificateTrustStatus {
    CertificateTrustStatus {
        state: CertificateTrustState::RootMissing,
        root_path: Some(root.display().to_string()),
        message: "Caddy 로컬 CA 파일이 아직 없습니다. Caddy를 한 번 시작하면 생성됩니다.".into(),
        node_hint: node_system_ca_hint(),
    }
}

fn root_crt_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(root_crt_path_from_home(Path::new(&home)))
}

fn root_crt_path_from_home(home: &Path) -> PathBuf {
    home.join(CADDY_DIR).join(ROOT_CRT_REL)
}

fn is_trusted_for_ssl(root: &Path) -> Result<bool> {
    let output = Command::new("/usr/bin/security")
        .arg("verify-cert")
        .arg("-c")
        .arg(root)
        .arg("-p")
        .arg("ssl")
        .arg("-L")
        .output()
        .context("spawn security verify-cert")?;

    Ok(output.status.success())
}

fn run_add_trusted_cert(root: &Path) -> Result<()> {
    let root_str = root
        .to_str()
        .ok_or_else(|| anyhow!("caddy root certificate path is not valid UTF-8"))?;
    let script = add_trusted_cert_applescript(root_str);

    let output = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .context("spawn osascript")?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        if is_user_cancelled(&err) {
            return Err(anyhow!("사용자가 관리자 권한 요청을 취소했습니다"));
        }
        return Err(anyhow!("security add-trusted-cert failed: {}", err));
    }

    Ok(())
}

fn add_trusted_cert_applescript(root: &str) -> String {
    format!(
        "do shell script \"/usr/bin/security add-trusted-cert -d -r trustRoot -p ssl -k {} \" & quoted form of \"{}\" with administrator privileges",
        SYSTEM_KEYCHAIN,
        escape_applescript_string(root)
    )
}

fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn is_user_cancelled(stderr: &str) -> bool {
    stderr.contains("User canceled") || stderr.contains("-128")
}

fn node_system_ca_hint() -> String {
    "Node 22.19+/24.6+에서는 NODE_USE_SYSTEM_CA=1, Node 23.8+에서는 --use-system-ca로 macOS 신뢰 저장소를 사용할 수 있습니다. Node 20은 기본적으로 macOS Keychain을 보지 않아 별도 설정이 필요할 수 있습니다.".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_path_uses_caddy_local_ca_location() {
        let path = root_crt_path_from_home(Path::new("/Users/me"));
        assert_eq!(
            path,
            PathBuf::from(
                "/Users/me/Library/Application Support/Caddy/pki/authorities/local/root.crt"
            )
        );
    }

    #[test]
    fn user_cancel_detection_matches_osascript_errors() {
        assert!(is_user_cancelled("execution error: User canceled. (-128)"));
        assert!(is_user_cancelled("error -128"));
        assert!(!is_user_cancelled("permission denied"));
    }

    #[test]
    fn applescript_escapes_certificate_path() {
        let script =
            add_trusted_cert_applescript("/Users/me/Library/Application Support/Caddy/root.crt");
        assert!(script.contains("quoted form of"));
        assert!(script.contains("/Users/me/Library/Application Support/Caddy/root.crt"));
    }
}

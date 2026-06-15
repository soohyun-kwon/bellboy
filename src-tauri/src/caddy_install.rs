//! Caddy 의존성 점검 및 Homebrew를 통한 자동 설치.
//!
//! Bellboy의 엔진은 Caddy 바이너리입니다. 미설치 상태면 프론트가 배너로 안내하고,
//! 사용자가 설치 버튼을 누르면 `brew install caddy`로 설치합니다.
//!
//! Homebrew 자체 자동 설치(`curl | bash` + sudo)는 위험하고 범위 밖이라
//! 다루지 않습니다. brew가 없으면 상태만 알려 프론트가 안내 문구를 띄웁니다.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;

/// GUI 런치에서 PATH에 누락되는 Homebrew 실행 경로.
const BREW_FALLBACKS: [&str; 2] = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"];

/// 실패 메시지에 포함할 brew 출력 tail의 최대 바이트 수.
const STDERR_TAIL_BYTES: usize = 800;

/// 프론트가 의존성 배너를 그리는 데 필요한 단일 스냅샷.
#[derive(Debug, Serialize)]
pub struct DependencyStatus {
    /// Caddy 바이너리가 실제로 존재하는지.
    pub caddy_installed: bool,
    /// 찾은 caddy 경로 (없으면 `None`).
    pub caddy_path: Option<String>,
    /// Homebrew(`brew`)가 설치되어 있는지 — 자동 설치 가능 여부.
    pub homebrew_installed: bool,
}

pub fn status() -> DependencyStatus {
    let caddy = crate::caddy_process::resolve_caddy();
    DependencyStatus {
        caddy_installed: caddy.is_some(),
        caddy_path: caddy.map(|p| p.display().to_string()),
        homebrew_installed: brew_path().is_some(),
    }
}

/// `brew install caddy`를 실행합니다. Homebrew가 없으면 즉시 실패합니다.
/// brew는 root 실행을 거부하므로 sudo 없이 현재 사용자 권한으로 돌립니다.
pub fn install_caddy() -> Result<()> {
    let brew = brew_path().ok_or_else(|| {
        anyhow!("Homebrew가 설치되어 있지 않습니다. https://brew.sh 에서 먼저 설치하세요.")
    })?;

    let output = Command::new(&brew)
        .args(["install", "caddy"])
        // 설치 전 전체 formulae 자동 갱신을 막아 시간을 단축합니다.
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_INSTALL_CLEANUP", "1")
        .output()
        .with_context(|| format!("spawn brew at {}", brew.display()))?;

    if output.status.success() {
        return Ok(());
    }

    // brew가 non-zero로 끝났더라도 caddy 바이너리가 실제로 깔렸다면 성공으로 본다.
    // post-install 경고나 cleanup 실패로 exit code만 비정상인 경우 거짓 실패를 막는다.
    if crate::caddy_process::resolve_caddy().is_some() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "caddy 설치 실패:\n{}",
        tail(&stderr, STDERR_TAIL_BYTES)
    ))
}

fn brew_path() -> Option<PathBuf> {
    crate::caddy_process::resolve_binary("brew", &BREW_FALLBACKS)
}

/// 문자열의 마지막 `max` 바이트 부근만 남깁니다. UTF-8 문자 경계를 보존하며,
/// 잘렸을 때는 앞에 `…`를 붙입니다.
fn tail(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.len() <= max {
        return trimmed.to_string();
    }
    let mut start = trimmed.len() - max;
    while start < trimmed.len() && !trimmed.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &trimmed[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_returns_full_string_when_short() {
        assert_eq!(tail("  hello  ", 800), "hello");
    }

    #[test]
    fn tail_truncates_long_string_with_ellipsis() {
        let s = "a".repeat(1000);
        let out = tail(&s, 800);
        assert!(out.starts_with('…'));
        // … (3 bytes) + 마지막 800 bytes
        assert_eq!(out.len(), '…'.len_utf8() + 800);
    }

    #[test]
    fn tail_preserves_utf8_boundary() {
        // 멀티바이트 문자로 채워 경계에서 잘려도 패닉하지 않는지 확인.
        let s = "가".repeat(500); // 각 3바이트 = 1500바이트
        let out = tail(&s, 800);
        assert!(out.starts_with('…'));
        // 잘린 본문은 온전한 '가'들로만 구성된다.
        assert!(out.trim_start_matches('…').chars().all(|c| c == '가'));
    }
}

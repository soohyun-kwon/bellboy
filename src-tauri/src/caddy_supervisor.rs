//! Detects caddy processes outside the one Bellboy currently owns, and tells
//! callers whether they were spawned by Bellboy (our Caddyfile in their argv) or
//! by something else (`brew services`, a manual `caddy run`, another tool).
//!
//! Why this exists: a force-quit or crashed Bellboy leaves its caddy child alive.
//! On the next launch, [`CaddyState::start`](caddy_process.rs) will happily
//! spawn another caddy, and the two race for ports 80/443/2019. That race is
//! what produced the `tlsv1 alert internal error` we observed in the wild.
//!
//! Detection uses `pgrep` + `ps` so we don't pull in `sysinfo` for one screen.

use anyhow::{anyhow, Result};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

/// What we found when we looked at running caddy processes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaddySighting {
    /// Nothing's running.
    None,
    /// Our tracked child is alive and well.
    OursAlive { pid: u32 },
    /// We had a pid file or tracked child, but it's gone — caller should clear stale state.
    OursDead,
    /// One or more caddy processes are running that we don't own.
    /// `bellboy_owned` are caddies started with Bellboy's Caddyfile (safe to auto-kill).
    /// `external` are caddies from other sources (need user confirmation).
    Foreign {
        bellboy_owned: Vec<ProcessInfo>,
        external: Vec<ProcessInfo>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub command: String,
}

/// Scans the process table and classifies what's running. `our_pid` should be
/// the PID of the [`CaddyState`]-owned child if any.
pub fn inspect(our_pid: Option<u32>, bellboy_caddyfile: &Path) -> CaddySighting {
    let pids = pgrep_caddy().unwrap_or_default();

    if let Some(ours) = our_pid {
        if !pids.contains(&ours) {
            return CaddySighting::OursDead;
        }
    }

    let pid_file = crate::caddy_process::read_pid_file();
    let owned_pid = our_pid.or(pid_file);

    let mut bellboy_owned = Vec::new();
    let mut external = Vec::new();
    let mut ours_alive = false;

    for pid in &pids {
        if Some(*pid) == owned_pid {
            ours_alive = true;
            continue;
        }
        let command = process_command(*pid).unwrap_or_default();
        let info = ProcessInfo { pid: *pid, command };
        if cmdline_uses_bellboy_caddyfile(&info.command, bellboy_caddyfile) {
            bellboy_owned.push(info);
        } else {
            external.push(info);
        }
    }

    if bellboy_owned.is_empty() && external.is_empty() {
        return match owned_pid {
            Some(pid) if ours_alive => CaddySighting::OursAlive { pid },
            Some(_) => CaddySighting::OursDead,
            None => CaddySighting::None,
        };
    }

    CaddySighting::Foreign {
        bellboy_owned,
        external,
    }
}

/// SIGTERM the listed PIDs. Best-effort: missing or unkillable processes are
/// skipped silently so partial cleanup still makes progress.
pub fn kill_pids(pids: &[u32]) -> Result<()> {
    if pids.is_empty() {
        return Ok(());
    }
    let mut args: Vec<String> = vec!["-TERM".into()];
    args.extend(pids.iter().map(|p| p.to_string()));
    let status = Command::new("/bin/kill")
        .args(&args)
        .status()
        .map_err(|e| anyhow!("spawn kill: {}", e))?;
    if !status.success() {
        return Err(anyhow!("kill returned non-zero status"));
    }
    Ok(())
}

/// TCP-connect probe to the Caddy admin API. We don't need an HTTP roundtrip
/// — if the admin socket isn't accepting connections, the proxy can't reload
/// and is by definition unhealthy.
pub fn admin_api_reachable() -> bool {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    let addr: SocketAddr = match "127.0.0.1:2019".parse() {
        Ok(a) => a,
        Err(_) => return false,
    };
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

fn pgrep_caddy() -> Result<Vec<u32>> {
    let output = Command::new("/usr/bin/pgrep")
        .args(["-x", "caddy"])
        .output()
        .map_err(|e| anyhow!("spawn pgrep: {}", e))?;
    // pgrep exits 1 when no match — that's a normal "none" outcome, not an error.
    if !output.status.success() && !output.stdout.is_empty() {
        return Err(anyhow!("pgrep failed"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_pgrep_output(&stdout))
}

fn parse_pgrep_output(stdout: &str) -> Vec<u32> {
    stdout
        .lines()
        .filter_map(|l| l.trim().parse::<u32>().ok())
        .collect()
}

fn process_command(pid: u32) -> Result<String> {
    let output = Command::new("/bin/ps")
        .args(["-o", "command=", "-p", &pid.to_string()])
        .output()
        .map_err(|e| anyhow!("spawn ps: {}", e))?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn cmdline_uses_bellboy_caddyfile(command: &str, bellboy_caddyfile: &Path) -> bool {
    let Some(needle) = bellboy_caddyfile.to_str() else {
        return false;
    };
    if needle.is_empty() {
        return false;
    }
    command.contains(needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_pgrep_lines() {
        assert_eq!(parse_pgrep_output("1234\n5678\n"), vec![1234, 5678]);
        assert_eq!(parse_pgrep_output(""), Vec::<u32>::new());
        assert_eq!(parse_pgrep_output("  42  \n\n"), vec![42]);
    }

    #[test]
    fn classifies_bellboy_owned_via_caddyfile_path() {
        let bellboy_cf = PathBuf::from("/Users/me/Library/Application Support/bellboy/Caddyfile");
        let bellboy_cmd =
            "/opt/homebrew/bin/caddy run --config /Users/me/Library/Application Support/bellboy/Caddyfile --adapter caddyfile";
        let brew_cmd = "/opt/homebrew/opt/caddy/bin/caddy run --config /opt/homebrew/etc/Caddyfile";

        assert!(cmdline_uses_bellboy_caddyfile(bellboy_cmd, &bellboy_cf));
        assert!(!cmdline_uses_bellboy_caddyfile(brew_cmd, &bellboy_cf));
    }

    #[test]
    fn empty_path_is_never_bellboy_owned() {
        // Without the guard, `"any".contains("")` would be true and we'd
        // auto-kill every caddy on the box.
        let cf = PathBuf::from("");
        assert!(!cmdline_uses_bellboy_caddyfile("anything", &cf));
    }
}

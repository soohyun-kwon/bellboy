use anyhow::{anyhow, Result};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

/// File under `app_dir` recording the PID of the Caddy child we spawned, so that
/// a fresh Bellboy launch can tell its own caddy (which it should adopt or kill
/// cleanly) apart from a foreign caddy left behind by `brew services`, a manual
/// `caddy run`, or a previous crash.
const PID_FILENAME: &str = "caddy.pid";

/// Wraps a long-running `caddy run` child process and exposes start/stop/reload.
///
/// We invoke the `caddy` binary from PATH, with Homebrew fallbacks for macOS
/// GUI launches where `/opt/homebrew/bin` is often missing from PATH.
pub struct CaddyState {
    child: Mutex<Option<Child>>,
}

impl CaddyState {
    pub fn new() -> Self {
        Self {
            child: Mutex::new(None),
        }
    }

    pub fn start(&self, caddyfile: &Path) -> Result<()> {
        let mut guard = self.child.lock().unwrap();
        if let Some(existing) = guard.as_mut() {
            match existing.try_wait()? {
                Some(_) => { /* previous process exited — fall through and start again */ }
                None => return Err(anyhow!("Caddy is already running")),
            }
        }

        let caddy = caddy_binary();
        let mut child = Command::new(&caddy)
            .args([
                "run",
                "--config",
                caddyfile
                    .to_str()
                    .ok_or_else(|| anyhow!("caddyfile path is not valid UTF-8"))?,
                "--adapter",
                "caddyfile",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                anyhow!(
                    "failed to spawn caddy at {}: {} — is Caddy installed and in PATH?",
                    caddy.display(),
                    e
                )
            })?;

        // Pipe stdout/stderr to ~/Library/Application Support/bellboy/caddy.log so
        // that a sudden exit (e.g. port-bind permission denied) is visible to
        // the user instead of silently vanishing.
        if let Ok(log_path) = crate::config_store::app_dir().map(|d| d.join("caddy.log")) {
            // Write the session header once, before spinning up the workers, so
            // we don't get duplicate start markers (one per stream).
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                use std::io::Write;
                let _ = writeln!(file, "--- caddy start ({}) ---", now_unix_secs());
            }
            if let Some(stderr) = child.stderr.take() {
                spawn_log_writer(stderr, log_path.clone(), "stderr");
            }
            if let Some(stdout) = child.stdout.take() {
                spawn_log_writer(stdout, log_path, "stdout");
            }
        }

        let _ = write_pid_file(child.id());
        *guard = Some(child);
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        let mut guard = self.child.lock().unwrap();
        if let Some(mut child) = guard.take() {
            // Try a graceful shutdown via the admin API first so Caddy can
            // release its locks (PKI, autocert) cleanly. If the API doesn't
            // respond promptly, fall back to SIGKILL.
            if !graceful_admin_stop() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        let _ = remove_pid_file();
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        let mut guard = self.child.lock().unwrap();
        match guard.as_mut() {
            Some(child) => matches!(child.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// PID of the caddy child we own, if any is alive in our process table.
    pub fn current_pid(&self) -> Option<u32> {
        let mut guard = self.child.lock().unwrap();
        let child = guard.as_mut()?;
        match child.try_wait() {
            Ok(None) => Some(child.id()),
            _ => None,
        }
    }

    /// Hot-reloads Caddy via its admin API. Caller must have called `start` first.
    pub fn reload(&self, caddyfile: &Path) -> Result<()> {
        if !self.is_running() {
            return Ok(());
        }
        let status = Command::new(caddy_binary())
            .args([
                "reload",
                "--config",
                caddyfile
                    .to_str()
                    .ok_or_else(|| anyhow!("caddyfile path is not valid UTF-8"))?,
                "--adapter",
                "caddyfile",
                "--address",
                "localhost:2019",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;

        if !status.status.success() {
            return Err(anyhow!(
                "caddy reload failed: {}",
                String::from_utf8_lossy(&status.stderr)
            ));
        }
        Ok(())
    }
}

/// Common Homebrew prefixes that macOS GUI launches strip from PATH.
const CADDY_FALLBACKS: [&str; 2] = ["/opt/homebrew/bin/caddy", "/usr/local/bin/caddy"];

/// Locates the `caddy` binary only if it actually exists on disk. Returns `None`
/// when Caddy isn't installed — the single source of truth for both the
/// installed-or-not check (`caddy_install`) and the spawn path (`caddy_binary`).
pub fn resolve_caddy() -> Option<PathBuf> {
    resolve_binary("caddy", &CADDY_FALLBACKS)
}

fn caddy_binary() -> PathBuf {
    resolve_caddy().unwrap_or_else(|| PathBuf::from("caddy"))
}

/// Finds `name` on PATH, then falls back to each absolute path in turn, returning
/// the first that exists. GUI app launches don't inherit the user's shell PATH,
/// so tools installed under Homebrew (`caddy`, `brew`) need these fallbacks.
pub(crate) fn resolve_binary(name: &str, fallbacks: &[&str]) -> Option<PathBuf> {
    if let Some(path) = find_in_path(name) {
        return Some(path);
    }
    fallbacks
        .iter()
        .map(|p| PathBuf::from(*p))
        .find(|path| path.is_file())
}

fn find_in_path(binary_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;

    std::env::split_paths(&path_var)
        .map(|dir| dir.join(binary_name))
        .find(|path| path.is_file())
}

impl Drop for CaddyState {
    fn drop(&mut self) {
        // Best-effort cleanup when the app exits.
        let _ = self.stop();
    }
}

/// Best-effort summary of why Caddy exited, pulled from `caddy.log`.
/// Prefers the `Error:` line from the most recent session; falls back to the
/// tail of that session's output. Returns `None` if the log is absent or empty.
pub fn recent_failure_summary() -> Option<String> {
    let log_path = crate::config_store::app_dir().ok()?.join("caddy.log");
    let content = std::fs::read_to_string(&log_path).ok()?;
    extract_failure_summary(&content, 20)
}

fn extract_failure_summary(content: &str, fallback_lines: usize) -> Option<String> {
    // Scope to the most recent session so old errors don't leak through.
    let haystack = match content.rfind("--- caddy start") {
        Some(i) => &content[i..],
        None => content,
    };

    // Caddy's canonical failure format starts with `Error:` — surface that
    // exact line when present. Falls back to a short tail otherwise.
    if let Some(line) = haystack.lines().rev().find(|l| l.contains("Error:")) {
        return Some(line.trim().to_string());
    }

    let lines: Vec<&str> = haystack.lines().collect();
    let start = lines.len().saturating_sub(fallback_lines);
    let tail = lines[start..].join("\n");
    if tail.trim().is_empty() {
        None
    } else {
        Some(tail)
    }
}

fn spawn_log_writer<R>(reader: R, log_path: std::path::PathBuf, label: &'static str)
where
    R: std::io::Read + Send + 'static,
{
    std::thread::spawn(move || {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path);
        let Ok(mut file) = file else { return };
        use std::io::Write;
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            let _ = writeln!(file, "[{}] {}", label, line);
        }
    });
}

/// Asks Caddy to shut itself down via the admin API. Returns `true` if Caddy
/// accepted the request — caller can then `wait()` on the child without sending
/// SIGKILL. Returns `false` on any failure so the caller falls back to kill().
///
/// We POST to `:2019/stop` with a short timeout: a hung admin API would
/// otherwise deadlock app shutdown.
fn graceful_admin_stop() -> bool {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let Ok(addr) = "127.0.0.1:2019".parse::<SocketAddr>() else {
        return false;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(150)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(150)));

    let request = "POST /stop HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 64];
    let Ok(n) = stream.read(&mut buf) else {
        return false;
    };
    let head = std::str::from_utf8(&buf[..n]).unwrap_or("");
    head.starts_with("HTTP/1.1 200") || head.starts_with("HTTP/1.0 200")
}

pub fn pid_file_path() -> Option<PathBuf> {
    crate::config_store::app_dir().ok().map(|d| d.join(PID_FILENAME))
}

/// Reads the PID file written at last successful `start`. `None` if the file is
/// missing or unparseable — callers must still verify the PID is actually alive.
pub fn read_pid_file() -> Option<u32> {
    let path = pid_file_path()?;
    let text = std::fs::read_to_string(&path).ok()?;
    text.trim().parse().ok()
}

pub fn remove_pid_file() -> std::io::Result<()> {
    let Some(path) = pid_file_path() else {
        return Ok(());
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn write_pid_file(pid: u32) -> std::io::Result<()> {
    let Some(path) = pid_file_path() else {
        return Ok(());
    };
    std::fs::write(&path, pid.to_string())
}

/// Tiny UNIX-seconds timestamp to avoid pulling `chrono` just for this.
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_error_line_over_tail() {
        let log = "\
--- caddy start (1) ---
[stderr] info: started admin
[stderr] Error: bind: address already in use
[stderr] info: shutdown
";
        assert_eq!(
            extract_failure_summary(log, 5).as_deref(),
            Some("[stderr] Error: bind: address already in use"),
        );
    }

    #[test]
    fn falls_back_to_tail_when_no_error_marker() {
        let log = "\
--- caddy start (1) ---
[stderr] line1
[stderr] line2
[stderr] line3
";
        let out = extract_failure_summary(log, 2).unwrap();
        assert!(out.contains("line2"));
        assert!(out.contains("line3"));
        assert!(!out.contains("line1"));
    }

    #[test]
    fn only_considers_latest_session() {
        let log = "\
--- caddy start (1) ---
[stderr] Error: old error
--- caddy start (2) ---
[stderr] info: running
[stderr] Error: new error
";
        let out = extract_failure_summary(log, 10).unwrap();
        assert!(out.contains("new error"));
        assert!(!out.contains("old error"));
    }

    #[test]
    fn empty_input_returns_none() {
        assert_eq!(extract_failure_summary("", 5), None);
        assert_eq!(extract_failure_summary("\n\n", 5), None);
    }
}

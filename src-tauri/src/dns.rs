//! External DNS resolver that bypasses `/etc/hosts`.
//!
//! Why this exists: Bellboy adds `127.0.0.1 <domain>` entries to /etc/hosts for
//! every managed site. If the user also configures a proxy rule whose target
//! is that same domain (the "local frontend + real remote API" pattern), the
//! OS resolver will return 127.0.0.1 and Caddy will loop back to itself.
//!
//! We sidestep this by shelling out to `dig` against a public resolver. `dig`
//! ships with macOS, so there's no extra dependency.

use anyhow::{anyhow, Context, Result};
use std::process::Command;
use std::time::Duration;

/// Public resolvers we'll query in order. Using more than one gives us a
/// fallback if a particular resolver is blocked or slow.
const RESOLVERS: &[&str] = &["8.8.8.8", "1.1.1.1"];

/// Per-query timeout. Kept short so the UI doesn't stall when offline.
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);

/// Resolve `host` to a single IPv4 address via a public DNS server,
/// ignoring the local hosts file. Returns the first A record found.
pub fn resolve_external(host: &str) -> Result<String> {
    if host.is_empty() {
        return Err(anyhow!("empty host"));
    }

    let mut last_err: Option<anyhow::Error> = None;
    for resolver in RESOLVERS {
        match query(host, resolver) {
            Ok(ip) => return Ok(ip),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("no resolvers configured")))
}

fn query(host: &str, resolver: &str) -> Result<String> {
    let timeout_secs = QUERY_TIMEOUT.as_secs().max(1).to_string();
    let at = format!("@{}", resolver);
    let time_opt = format!("+time={}", timeout_secs);

    let output = Command::new("dig")
        .args([
            "+short",
            "+tries=1",
            time_opt.as_str(),
            at.as_str(),
            host,
            "A",
        ])
        .output()
        .context("spawn dig")?;

    if !output.status.success() {
        return Err(anyhow!(
            "dig exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    first_ipv4(&stdout).ok_or_else(|| anyhow!("no A record returned for {host}"))
}

/// Pick the first line that looks like a dotted-quad IPv4.
/// `dig +short` may emit a CNAME line before A records, which we skip.
fn first_ipv4(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| is_ipv4(line))
        .map(String::from)
}

fn is_ipv4(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts
        .iter()
        .all(|p| !p.is_empty() && p.parse::<u8>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_detection() {
        assert!(is_ipv4("127.0.0.1"));
        assert!(is_ipv4("8.8.8.8"));
        assert!(!is_ipv4("example.com."));
        assert!(!is_ipv4("1.2.3"));
        assert!(!is_ipv4("1.2.3.4.5"));
        assert!(!is_ipv4(""));
        assert!(!is_ipv4("1.2.3.300"));
    }

    #[test]
    fn first_ipv4_skips_cname() {
        let out = "example.com.\n1.2.3.4\n5.6.7.8\n";
        assert_eq!(first_ipv4(out).as_deref(), Some("1.2.3.4"));
    }

    #[test]
    fn first_ipv4_empty() {
        assert_eq!(first_ipv4(""), None);
        assert_eq!(first_ipv4("\n\n"), None);
    }
}

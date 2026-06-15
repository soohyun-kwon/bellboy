/// Manages `NODE_EXTRA_CA_CERTS` for the user's GUI session so that Node.js
/// server-side code (e.g. Next.js SSR) trusts HTTPS requests to Caddy-managed
/// local domains like `dev.lfind.io.kr`.
///
/// Root cause: Node.js uses its own bundled CA list, not the macOS keychain.
/// Caddy's `tls internal` certs are signed by Caddy's local CA, stored at
/// `~/Library/Application Support/Caddy/pki/authorities/local/root.crt`.
/// This CA is NOT in the macOS system bundle (/etc/ssl/cert.pem), so Node
/// rejects TLS connections to local Caddy domains with
/// `UNABLE_TO_GET_ISSUER_CERT_LOCALLY`.
///
/// Fix: create a combined PEM bundle = system CAs + Caddy local CA, then
/// point `NODE_EXTRA_CA_CERTS` at that bundle. Rebuilt every time Caddy starts
/// (in case Caddy ever regenerates its CA).
use anyhow::{anyhow, Result};
use std::path::PathBuf;
use std::process::Command;

const LABEL: &str = "io.bellboy.node-tls";
const ENV_VAR: &str = "NODE_EXTRA_CA_CERTS";

// macOS system CA bundle (public roots — needed for external HTTPS calls too)
const SYSTEM_CA_PATH: &str = "/etc/ssl/cert.pem";

fn plist_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME not set"))?;
    Ok(PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", LABEL)))
}

fn caddy_root_cert_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home)
        .join("Library/Application Support/Caddy/pki/authorities/local/root.crt");
    path.exists().then_some(path)
}

fn bundle_path() -> Result<PathBuf> {
    Ok(crate::config_store::app_dir()?.join("ca-bundle.pem"))
}

/// Whether the LaunchAgent plist is installed (= feature is enabled).
pub fn is_enabled() -> bool {
    plist_path().map(|p| p.exists()).unwrap_or(false)
}

/// Rebuilds the combined CA bundle: system CAs + Caddy local root CA.
/// Called on enable AND after each Caddy start so the bundle stays fresh.
/// Returns the bundle path on success.
pub fn rebuild_bundle() -> Result<PathBuf> {
    let out = bundle_path()?;

    let mut content = String::new();

    // 1. macOS system public CA roots
    if let Ok(sys) = std::fs::read_to_string(SYSTEM_CA_PATH) {
        content.push_str(&sys);
    }

    // 2. Caddy local development CA (signs *.test, *.io.kr local domains, etc.)
    if let Some(caddy_root) = caddy_root_cert_path() {
        match std::fs::read_to_string(&caddy_root) {
            Ok(cert) => {
                content.push_str("\n# Caddy local CA (managed by Bellboy)\n");
                content.push_str(&cert);
            }
            Err(e) => {
                // Non-fatal: bundle still usable for public CAs
                eprintln!("[bellboy] warning: could not read Caddy root cert: {e}");
            }
        }
    } else {
        eprintln!("[bellboy] warning: Caddy root cert not found — start Caddy first");
    }

    std::fs::write(&out, content)?;
    Ok(out)
}

/// Writes the LaunchAgent plist and applies the env var to the current session.
/// Also injects into ~/.zshenv so terminal-launched processes (pnpm dev, etc.)
/// pick it up without needing a full logout/login.
pub fn enable() -> Result<()> {
    let bundle = rebuild_bundle()?;
    let bundle_str = bundle
        .to_str()
        .ok_or_else(|| anyhow!("bundle path is not valid UTF-8"))?;

    let plist = plist_path()?;
    if let Some(parent) = plist.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&plist, plist_content(bundle_str))?;

    // Load so it takes effect for GUI apps without a logout.
    let plist_str = plist.to_string_lossy();
    let _ = Command::new("launchctl")
        .args(["load", "-w", &plist_str])
        .output();

    // Set immediately in the launchd session env.
    let _ = Command::new("launchctl")
        .args(["setenv", ENV_VAR, bundle_str])
        .output();

    // Also write to ~/.zshenv so terminal-spawned Node processes inherit it.
    let _ = zshenv_add(bundle_str);

    Ok(())
}

/// Removes the LaunchAgent plist, clears the env var, and removes the ~/.zshenv entry.
pub fn disable() -> Result<()> {
    let plist = plist_path()?;
    if plist.exists() {
        let plist_str = plist.to_string_lossy();
        let _ = Command::new("launchctl")
            .args(["unload", "-w", &plist_str])
            .output();
        std::fs::remove_file(&plist)?;
    }

    let _ = Command::new("launchctl")
        .args(["unsetenv", ENV_VAR])
        .output();

    let _ = zshenv_remove();

    Ok(())
}

const ZSHENV_MARKER: &str = "# bellboy:node-tls";

fn zshenv_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").map_err(|_| anyhow!("HOME not set"))?;
    Ok(PathBuf::from(home).join(".zshenv"))
}

fn zshenv_add(bundle_str: &str) -> Result<()> {
    let path = zshenv_path()?;
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    // Already present — update the export line in place.
    if existing.contains(ZSHENV_MARKER) {
        let updated = existing
            .lines()
            .map(|l| {
                if l.starts_with("export NODE_EXTRA_CA_CERTS=")
                    && existing.contains(ZSHENV_MARKER)
                {
                    format!("export {ENV_VAR}=\"{bundle_str}\"")
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, updated)?;
        return Ok(());
    }

    // Append new block.
    let block = format!(
        "\n{marker}\nexport {var}=\"{path}\"\n",
        marker = ZSHENV_MARKER,
        var = ENV_VAR,
        path = bundle_str,
    );
    let mut content = existing;
    content.push_str(&block);
    std::fs::write(&path, content)?;
    Ok(())
}

fn zshenv_remove() -> Result<()> {
    let path = zshenv_path()?;
    let existing = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };

    if !existing.contains(ZSHENV_MARKER) {
        return Ok(());
    }

    let filtered: Vec<&str> = existing
        .lines()
        .filter(|l| !l.starts_with(ZSHENV_MARKER) && !l.starts_with(&format!("export {ENV_VAR}=")))
        .collect();
    std::fs::write(&path, filtered.join("\n") + "\n")?;
    Ok(())
}

fn plist_content(bundle_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/launchctl</string>
        <string>setenv</string>
        <string>{env_var}</string>
        <string>{bundle_path}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
        label = LABEL,
        env_var = ENV_VAR,
        bundle_path = bundle_path,
    )
}

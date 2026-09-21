//! Shared status-api + MCP install (remote payload and embedded full apply).

use super::bins::{resolve_local_ecosystem_bins, LocalBins};
use super::process::{ProcessRunner, StdioMode};
use crate::error::{HortoError, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Default install prefix for box binaries and unit `ExecStart` paths.
pub const DEFAULT_INSTALL_DIR: &str = "/usr/local/bin";

/// Which ecosystem services to install (user opt-in, default neither).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EcosystemInstallChoice {
    /// Install/enable `horto-os-ui-status-api`.
    pub status_api: bool,
    /// Install/enable `horto-os-ui-mcp`.
    pub mcp: bool,
}

impl EcosystemInstallChoice {
    /// Neither service.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            status_api: false,
            mcp: false,
        }
    }

    /// True when at least one service is selected.
    #[must_use]
    pub const fn any(self) -> bool {
        self.status_api || self.mcp
    }
}
/// Status-api systemd unit body (`ExecStart` defaults to `/usr/local/bin`).
pub const STATUS_API_UNIT: &str = r#"[Unit]
Description=Horto OS UI status API
After=network.target

[Service]
Type=simple
EnvironmentFile=/etc/horto-os-ui/api.env
ExecStart=/usr/local/bin/horto-os-ui-status-api --bind 0.0.0.0:8787
Restart=on-failure

[Install]
WantedBy=multi-user.target
"#;

/// MCP systemd unit from packaging (kept in sync via `include_str!`).
pub const MCP_UNIT: &str =
    include_str!("../../../horto-os-ui-mcp/packaging/horto-os-ui-mcp.service");

/// User-owned drop file basename for token capture (remote).
pub const API_TOKEN_DROP_BASENAME: &str = ".horto-os-ui-api-token";

/// Ensure `/etc/horto-os-ui/api.env` exists with a random bearer token (0600).
///
/// Reuses an existing file so reinstall does not rotate the token. Writes the
/// hex into `$HOME/.horto-os-ui-api-token` (0600) for PC-side Capture (no TTY echo).
pub const ENSURE_API_TOKEN_SCRIPT: &str = r#"
set -e
sudo mkdir -p /etc/horto-os-ui
if [ ! -f /etc/horto-os-ui/api.env ]; then
  TOKEN=$(openssl rand -hex 32 2>/dev/null || head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')
  printf 'HORTO_API_TOKEN=%s\n' "$TOKEN" | sudo tee /etc/horto-os-ui/api.env >/dev/null
  sudo chmod 600 /etc/horto-os-ui/api.env
fi
DROP="${HOME}/.horto-os-ui-api-token"
sudo grep '^HORTO_API_TOKEN=' /etc/horto-os-ui/api.env > "$DROP"
chmod 600 "$DROP"
"#;

/// Rewrite default `/usr/local/bin/` ExecStart paths for a custom install prefix.
#[must_use]
pub fn unit_with_install_dir(unit: &str, install_dir: &str) -> String {
    let install = install_dir.trim_end_matches('/');
    if install == DEFAULT_INSTALL_DIR {
        return unit.to_owned();
    }
    unit.replace("/usr/local/bin/", &format!("{install}/"))
}

/// Shell fragment: install selected bins/units from `staging`.
///
/// Always installs CLI + TUI. Adds status-api / MCP bins and units per `choice`.
/// `staging` already contains the binary basenames.
#[must_use]
pub fn remote_enable_ecosystem_script(
    staging: &str,
    install_dir: &str,
    choice: EcosystemInstallChoice,
) -> String {
    let install = install_dir.trim_end_matches('/');
    let staging = staging.trim_end_matches('/');
    let api_unit_path = "/etc/systemd/system/horto-os-ui-status-api.service";
    let mcp_unit_path = "/etc/systemd/system/horto-os-ui-mcp.service";
    let api_unit = unit_with_install_dir(STATUS_API_UNIT, install);
    let mcp_unit = unit_with_install_dir(MCP_UNIT, install);

    let mut bin_names = vec!["horto-os-ui", "horto-os-ui-tui"];
    if choice.status_api {
        bin_names.push("horto-os-ui-status-api");
    }
    if choice.mcp {
        bin_names.push("horto-os-ui-mcp");
    }
    let bins = bin_names
        .iter()
        .map(|n| format!("{staging}/{n}"))
        .collect::<Vec<_>>()
        .join(" ");

    let mut script = String::new();
    if choice.status_api {
        script.push_str(ENSURE_API_TOKEN_SCRIPT);
        script.push('\n');
        script.push_str(&format!(
            "sudo tee {api_unit_path} > /dev/null <<'HORTO_UNIT_EOF'\n{api_unit}HORTO_UNIT_EOF\n"
        ));
    }
    if choice.mcp {
        script.push_str(&format!(
            "sudo tee {mcp_unit_path} > /dev/null <<'HORTO_UNIT_EOF'\n{mcp_unit}HORTO_UNIT_EOF\n"
        ));
    }
    script.push_str(&format!("sudo install -m 755 {bins} {install}/"));
    script.push_str(" && sudo systemctl daemon-reload");
    if choice.status_api {
        script.push_str(" && sudo systemctl enable --now horto-os-ui-status-api.service");
    }
    if choice.mcp {
        script.push_str(" && sudo systemctl enable --now horto-os-ui-mcp.service");
    }
    if install != DEFAULT_INSTALL_DIR {
        script.push_str(" && sudo systemctl daemon-reload");
        if choice.status_api {
            script.push_str(" && sudo systemctl restart horto-os-ui-status-api.service");
        }
        if choice.mcp {
            script.push_str(" && sudo systemctl restart horto-os-ui-mcp.service");
        }
    }
    script
}
fn api_env_path(etc_root: &Path) -> PathBuf {
    etc_root.join("horto-os-ui").join("api.env")
}

fn ensure_api_env_at(etc_root: &Path) -> Result<Option<String>> {
    let dir = etc_root.join("horto-os-ui");
    fs::create_dir_all(&dir)
        .map_err(|e| HortoError::msg(format!("create {}: {e}", dir.display())))?;
    let env_path = api_env_path(etc_root);
    if !env_path.is_file() {
        let token = random_hex_token();
        write_mode_600(&env_path, &format!("HORTO_API_TOKEN={token}\n"))?;
    }
    let raw = fs::read_to_string(&env_path)
        .map_err(|e| HortoError::msg(format!("read {}: {e}", env_path.display())))?;
    Ok(parse_api_token_line(&raw))
}

fn parse_api_token_line(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let hex = line.strip_prefix("HORTO_API_TOKEN=").unwrap_or(line).trim();
    if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(hex.to_owned())
}

fn random_hex_token() -> String {
    use std::io::Read;
    let mut buf = [0u8; 32];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut buf);
    } else {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (i, b) in buf.iter_mut().enumerate() {
            *b = ((nanos >> ((i % 16) * 8)) & 0xff) as u8;
        }
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn write_mode_600(path: &Path, body: &str) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| HortoError::msg(format!("write {}: {e}", path.display())))?;
        f.write_all(body.as_bytes())
            .map_err(|e| HortoError::msg(format!("write {}: {e}", path.display())))?;
    }
    #[cfg(not(unix))]
    {
        fs::write(path, body)
            .map_err(|e| HortoError::msg(format!("write {}: {e}", path.display())))?;
    }
    Ok(())
}

fn write_unit_file(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| HortoError::msg(format!("create {}: {e}", parent.display())))?;
    }
    fs::write(path, body).map_err(|e| HortoError::msg(format!("write {}: {e}", path.display())))?;
    Ok(())
}

fn install_bin(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| HortoError::msg(format!("create {}: {e}", parent.display())))?;
    }
    fs::copy(src, dest).map_err(|e| {
        HortoError::msg(format!(
            "install {} → {}: {e}",
            src.display(),
            dest.display()
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dest, fs::Permissions::from_mode(0o755))
            .map_err(|e| HortoError::msg(format!("chmod {}: {e}", dest.display())))?;
    }
    Ok(())
}

/// Install selected ecosystem bins into `install_dir` and enable chosen units.
///
/// Always copies CLI + TUI. Copies status-api / MCP and enables units per `choice`.
/// Intended for embedded full apply (already root).
///
/// # Errors
///
/// Returns [`crate::HortoError`] when install or `systemctl` fails.
pub fn install_ecosystem_services(
    runner: &dyn ProcessRunner,
    bins: &LocalBins,
    install_dir: &Path,
    choice: EcosystemInstallChoice,
) -> Result<Option<String>> {
    install_ecosystem_services_at(runner, bins, install_dir, choice, Path::new("/etc"))
}

fn install_ecosystem_services_at(
    runner: &dyn ProcessRunner,
    bins: &LocalBins,
    install_dir: &Path,
    choice: EcosystemInstallChoice,
    etc_root: &Path,
) -> Result<Option<String>> {
    if !choice.any() {
        return Ok(None);
    }
    let install = install_dir
        .to_str()
        .ok_or_else(|| HortoError::msg("non-utf8 install_dir"))?;

    let mut token = None;
    if choice.status_api {
        token = ensure_api_env_at(etc_root)?;
    }

    let mut pairs: Vec<(&PathBuf, &str)> =
        vec![(&bins.cli, "horto-os-ui"), (&bins.tui, "horto-os-ui-tui")];
    if choice.status_api {
        pairs.push((&bins.status_api, "horto-os-ui-status-api"));
    }
    if choice.mcp {
        pairs.push((&bins.mcp, "horto-os-ui-mcp"));
    }
    for (src, name) in pairs {
        let dest = install_dir.join(name);
        if src != &dest || !dest.is_file() {
            install_bin(src, &dest)?;
        }
    }

    let unit_dir = etc_root.join("systemd").join("system");
    if choice.status_api {
        let api_unit = unit_with_install_dir(STATUS_API_UNIT, install);
        write_unit_file(&unit_dir.join("horto-os-ui-status-api.service"), &api_unit)?;
    }
    if choice.mcp {
        let mcp_unit = unit_with_install_dir(MCP_UNIT, install);
        write_unit_file(&unit_dir.join("horto-os-ui-mcp.service"), &mcp_unit)?;
    }

    let mut sys_args: Vec<&[&str]> = vec![&["daemon-reload"]];
    if choice.status_api {
        sys_args.push(&["enable", "--now", "horto-os-ui-status-api.service"]);
    }
    if choice.mcp {
        sys_args.push(&["enable", "--now", "horto-os-ui-mcp.service"]);
    }
    for args in sys_args {
        let out = runner.run("systemctl", args, &[], StdioMode::Capture)?;
        if !out.success() {
            let detail = if out.stderr.trim().is_empty() {
                out.stdout.trim()
            } else {
                out.stderr.trim()
            };
            return Err(HortoError::command(
                "systemctl",
                format!("exit {}: {detail}", out.status),
            ));
        }
    }

    Ok(token)
}

/// Resolve local bins and install selected ecosystem services after embedded full apply.
///
/// # Errors
///
/// Returns [`crate::HortoError`] when bins cannot be resolved or install fails.
pub fn install_ecosystem_after_embedded_apply(
    runner: &dyn ProcessRunner,
    install_dir: &Path,
    choice: EcosystemInstallChoice,
) -> Result<Option<String>> {
    if !choice.any() {
        return Ok(None);
    }
    let bins = resolve_local_ecosystem_bins(install_dir)?;
    install_ecosystem_services(runner, &bins, install_dir, choice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::bins::BOX_BIN_NAMES;
    use crate::remote::process::ScriptedRunner;
    use tempfile::TempDir;

    #[test]
    fn unit_rewrite_custom_prefix() {
        let u = unit_with_install_dir(STATUS_API_UNIT, "/opt/horto/bin");
        assert!(u.contains("/opt/horto/bin/horto-os-ui-status-api"));
        assert!(!u.contains("/usr/local/bin/horto-os-ui-status-api"));
        let m = unit_with_install_dir(MCP_UNIT, "/opt/horto/bin");
        assert!(m.contains("/opt/horto/bin/horto-os-ui-mcp"));
        assert!(MCP_UNIT.contains("horto-os-ui-status-api.service"));
    }

    #[test]
    fn remote_enable_script_mentions_both_units_and_mcp_bin() {
        let both = EcosystemInstallChoice {
            status_api: true,
            mcp: true,
        };
        let s = remote_enable_ecosystem_script("/tmp/stage", "/usr/local/bin", both);
        assert!(s.contains("horto-os-ui-status-api.service"));
        assert!(s.contains("horto-os-ui-mcp.service"));
        assert!(s.contains("horto-os-ui-mcp"));
        assert!(s.contains("enable --now horto-os-ui-mcp.service"));
        assert!(!s.contains("sudo sed"));
    }

    #[test]
    fn remote_enable_script_api_only_skips_mcp_unit() {
        let api = EcosystemInstallChoice {
            status_api: true,
            mcp: false,
        };
        let s = remote_enable_ecosystem_script("/tmp/stage", "/usr/local/bin", api);
        assert!(s.contains("horto-os-ui-status-api.service"));
        assert!(!s.contains("horto-os-ui-mcp.service"));
    }

    #[test]
    fn remote_enable_script_custom_prefix_restarts() {
        let both = EcosystemInstallChoice {
            status_api: true,
            mcp: true,
        };
        let s = remote_enable_ecosystem_script("/tmp/stage", "/opt/horto/bin", both);
        assert!(s.contains("/opt/horto/bin/horto-os-ui-mcp"));
        assert!(s.contains("restart horto-os-ui-mcp.service"));
    }

    #[test]
    fn remote_enable_script_mcp_only_skips_api_token() {
        let mcp = EcosystemInstallChoice {
            status_api: false,
            mcp: true,
        };
        let s = remote_enable_ecosystem_script("/tmp/stage", "/usr/local/bin", mcp);
        assert!(!s.contains("openssl rand"));
        assert!(!s.contains(API_TOKEN_DROP_BASENAME));
        assert!(s.contains("horto-os-ui-mcp.service"));
        assert!(!s.contains("enable --now horto-os-ui-status-api.service"));
    }

    #[test]
    fn parse_api_token_line_accepts_and_rejects() {
        assert_eq!(
            parse_api_token_line("HORTO_API_TOKEN=aabb\n").as_deref(),
            Some("aabb")
        );
        assert_eq!(
            parse_api_token_line("deadbeef\n").as_deref(),
            Some("deadbeef")
        );
        assert!(parse_api_token_line("").is_none());
        assert!(parse_api_token_line("HORTO_API_TOKEN=\n").is_none());
        assert!(parse_api_token_line("not-hex!\n").is_none());
    }

    #[test]
    fn choice_helpers() {
        assert!(!EcosystemInstallChoice::none().any());
        assert!(EcosystemInstallChoice {
            status_api: true,
            mcp: false
        }
        .any());
        assert!(EcosystemInstallChoice {
            status_api: false,
            mcp: true
        }
        .any());
    }

    #[test]
    fn install_ecosystem_services_none_is_noop() {
        let tmp = TempDir::new().unwrap();
        let bins = LocalBins {
            dir: tmp.path().to_path_buf(),
            cli: tmp.path().join("horto-os-ui"),
            tui: tmp.path().join("horto-os-ui-tui"),
            status_api: tmp.path().join("horto-os-ui-status-api"),
            mcp: tmp.path().join("horto-os-ui-mcp"),
        };
        let runner = ScriptedRunner::default();
        let token = install_ecosystem_services_at(
            &runner,
            &bins,
            tmp.path(),
            EcosystemInstallChoice::none(),
            tmp.path().join("etc").as_path(),
        )
        .unwrap();
        assert!(token.is_none());
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn install_ecosystem_services_copies_and_enables_both() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dest = tmp.path().join("dest");
        let etc = tmp.path().join("etc");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();
        for name in BOX_BIN_NAMES {
            fs::write(src.join(name), b"bin").unwrap();
        }
        let bins = LocalBins {
            dir: src.clone(),
            cli: src.join("horto-os-ui"),
            tui: src.join("horto-os-ui-tui"),
            status_api: src.join("horto-os-ui-status-api"),
            mcp: src.join("horto-os-ui-mcp"),
        };
        let runner = ScriptedRunner::default();
        runner.push("systemctl", ScriptedRunner::ok(""));
        runner.push("systemctl", ScriptedRunner::ok(""));
        runner.push("systemctl", ScriptedRunner::ok(""));

        let token = install_ecosystem_services_at(
            &runner,
            &bins,
            &dest,
            EcosystemInstallChoice {
                status_api: true,
                mcp: true,
            },
            &etc,
        )
        .unwrap();
        assert!(token.is_some());
        assert!(token
            .as_ref()
            .unwrap()
            .chars()
            .all(|c| c.is_ascii_hexdigit()));
        for name in BOX_BIN_NAMES {
            assert!(dest.join(name).is_file(), "missing {name}");
        }
        assert!(etc
            .join("systemd/system/horto-os-ui-status-api.service")
            .is_file());
        assert!(etc.join("systemd/system/horto-os-ui-mcp.service").is_file());
        assert!(etc.join("horto-os-ui/api.env").is_file());
        // Reuse token on second install
        let runner2 = ScriptedRunner::default();
        runner2.push("systemctl", ScriptedRunner::ok(""));
        runner2.push("systemctl", ScriptedRunner::ok(""));
        runner2.push("systemctl", ScriptedRunner::ok(""));
        let token2 = install_ecosystem_services_at(
            &runner2,
            &bins,
            &dest,
            EcosystemInstallChoice {
                status_api: true,
                mcp: true,
            },
            &etc,
        )
        .unwrap();
        assert_eq!(token, token2);
    }

    #[test]
    fn install_ecosystem_services_mcp_only_skips_api_env() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dest = tmp.path().join("dest");
        let etc = tmp.path().join("etc");
        fs::create_dir_all(&src).unwrap();
        for name in BOX_BIN_NAMES {
            fs::write(src.join(name), b"bin").unwrap();
        }
        let bins = LocalBins {
            dir: src.clone(),
            cli: src.join("horto-os-ui"),
            tui: src.join("horto-os-ui-tui"),
            status_api: src.join("horto-os-ui-status-api"),
            mcp: src.join("horto-os-ui-mcp"),
        };
        let runner = ScriptedRunner::default();
        runner.push("systemctl", ScriptedRunner::ok(""));
        runner.push("systemctl", ScriptedRunner::ok(""));

        let token = install_ecosystem_services_at(
            &runner,
            &bins,
            &dest,
            EcosystemInstallChoice {
                status_api: false,
                mcp: true,
            },
            &etc,
        )
        .unwrap();
        assert!(token.is_none());
        assert!(dest.join("horto-os-ui-mcp").is_file());
        assert!(!dest.join("horto-os-ui-status-api").exists());
        assert!(!etc.join("horto-os-ui/api.env").exists());
        assert!(etc.join("systemd/system/horto-os-ui-mcp.service").is_file());
        assert!(!etc
            .join("systemd/system/horto-os-ui-status-api.service")
            .exists());
    }

    #[test]
    fn install_ecosystem_services_systemctl_failure() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dest = tmp.path().join("dest");
        let etc = tmp.path().join("etc");
        fs::create_dir_all(&src).unwrap();
        for name in BOX_BIN_NAMES {
            fs::write(src.join(name), b"bin").unwrap();
        }
        let bins = LocalBins {
            dir: src.clone(),
            cli: src.join("horto-os-ui"),
            tui: src.join("horto-os-ui-tui"),
            status_api: src.join("horto-os-ui-status-api"),
            mcp: src.join("horto-os-ui-mcp"),
        };
        let runner = ScriptedRunner::default();
        runner.push("systemctl", ScriptedRunner::fail(1, "nope"));

        let err = install_ecosystem_services_at(
            &runner,
            &bins,
            &dest,
            EcosystemInstallChoice {
                status_api: true,
                mcp: false,
            },
            &etc,
        )
        .unwrap_err();
        assert!(err.to_string().contains("systemctl"));
    }

    #[test]
    fn install_after_embedded_none_skips_resolve() {
        let tmp = TempDir::new().unwrap();
        let runner = ScriptedRunner::default();
        let token = install_ecosystem_after_embedded_apply(
            &runner,
            tmp.path(),
            EcosystemInstallChoice::none(),
        )
        .unwrap();
        assert!(token.is_none());
    }

    #[test]
    fn random_hex_token_is_64_hex_chars() {
        let t = random_hex_token();
        assert_eq!(t.len(), 64);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }
}

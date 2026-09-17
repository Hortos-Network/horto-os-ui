//! reference: horto-os/scripts/d1_docker_init.sh
//! Multi-platform tip (`dev_multi-platform-os`): copy non-stacks tree, then
//! `stacks/common`, then `stacks/$NPU_TYPE` from `os-configuration.env`.
use crate::context::{HostContext, PlannedAction};
use crate::embed::{self, Assets};
use crate::error::{HortoError, Result};
use crate::kits::{envfile, fs, template};
use crate::step::Step;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct D1Docker;

impl Step for D1Docker {
    fn id(&self) -> &'static str {
        "d1"
    }
    fn title(&self) -> &'static str {
        "Initialize Docker stacks from embedded source"
    }
    fn reference_script(&self) -> &'static str {
        "d1_docker_init.sh"
    }
    fn step_version(&self) -> u32 {
        2
    }
    fn depends_on(&self) -> &'static [&'static str] {
        &[]
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        ctx.paths.docker.join("dockge/compose.yaml").exists()
            || ctx.paths.docker.join("stacks").is_dir()
            || ctx.paths.docker.join("homepage/compose.yaml").exists()
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        let npu = resolve_npu_type(ctx)?;
        ctx.plan_action(format!(
            "ensure {}",
            ctx.paths.os_configuration_file().display()
        ));
        ctx.plan_action(format!(
            "extract embedded docker_source/ (non-stacks) -> {}",
            ctx.paths.docker.display()
        ));
        ctx.plan_action("merge stacks/common into docker tree");
        match npu.as_str() {
            "none" => ctx.plan_action("NPU_TYPE=none; skip NPU/GPU stack merge"),
            other => ctx.plan_action(format!("merge stacks/{other} into docker tree")),
        }
        ctx.plan_action("copy homepage_assets/* into docker/assets");
        ctx.plan_action("render {{VAR}} placeholders under docker tree");
        if ctx.skip_piper {
            ctx.plan_action("skip piper model download (--skip-piper)");
        } else {
            ctx.plan_action("download piper models (warn-only on failure)");
        }
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        ensure_os_configuration(ctx)?;
        let vars = load_render_vars(ctx)?;
        let npu = resolve_npu_type(ctx)?;
        let target = ctx.paths.docker.clone();
        fs::ensure_dir(ctx, &target)?;

        extract_non_stacks(ctx, &target)?;
        merge_stack_prefix(ctx, &target, "stacks/common/")?;
        if npu != "none" {
            let prefix = format!("stacks/{npu}/");
            merge_stack_prefix(ctx, &target, &prefix)?;
        } else {
            ctx.log("NPU_TYPE=none; skipped NPU/GPU stack merge");
        }

        copy_homepage_assets(ctx)?;

        if ctx.is_dry_run() {
            ctx.plan_action("render placeholders in docker tree");
        } else {
            render_tree(ctx, &target, &vars)?;
        }

        if ctx.skip_piper || ctx.is_dry_run() {
            ctx.log("Skipping piper model download.");
        } else {
            download_piper_models(ctx);
        }

        ctx.log(format!(
            "d1 complete: docker source ready under {}",
            target.display()
        ));
        Ok(())
    }
}

fn ensure_os_configuration(ctx: &mut HostContext) -> Result<()> {
    let dest = ctx.paths.os_configuration_file();
    if dest.exists() {
        return Ok(());
    }
    let template = embed::get("config/os-configuration.env")
        .ok_or_else(|| HortoError::EmbedMissing("config/os-configuration.env".into()))?;
    fs::ensure_dir(ctx, &ctx.paths.active_setup.clone())?;
    fs::write_file(ctx, &dest, &template)?;
    if ctx.is_dry_run() {
        ctx.plan_action(format!("write {}", dest.display()));
    } else {
        ctx.log(format!("wrote {}", dest.display()));
    }
    Ok(())
}

/// Map WIP / alias values onto folders under `docker_source/stacks/`.
fn normalize_npu_type(raw: &str) -> String {
    match raw.trim() {
        "" | "none" => "none".into(),
        "rkRK3588" | "rk3588" | "RK3588" => "rk3588".into(),
        "rkRK3576" | "rk3576" | "RK3576" => "rk3576".into(),
        "no_wyoming" => "no_wyoming".into(),
        other => other.to_string(),
    }
}

fn resolve_npu_type(ctx: &HostContext) -> Result<String> {
    let path = ctx.paths.os_configuration_file();
    let map = if path.exists() {
        envfile::load(&path)?
    } else if ctx.is_dry_run() {
        let template = embed::get_str("config/os-configuration.env").unwrap_or_default();
        envfile::parse(&template)
    } else {
        return Err(HortoError::msg(format!(
            "missing {}; run setup so os-configuration is staged",
            path.display()
        )));
    };
    let raw = map.get("NPU_TYPE").map(String::as_str).unwrap_or("none");
    let npu = normalize_npu_type(raw);
    if npu == "none" {
        return Ok(npu);
    }
    let probe = format!("docker_source/stacks/{npu}/");
    let exists = Assets::iter().any(|p| p.starts_with(&probe));
    if !exists {
        return Err(HortoError::msg(format!(
            "NPU_TYPE={npu} has no embedded stacks/{npu}/ (fix OS_TYPE/NPU_TYPE in os-configuration.env)"
        )));
    }
    Ok(npu)
}

fn extract_non_stacks(ctx: &mut HostContext, target: &Path) -> Result<()> {
    for path in Assets::iter() {
        let Some(rel) = path.strip_prefix("docker_source/") else {
            continue;
        };
        if rel.is_empty() || rel.starts_with("stacks/") {
            continue;
        }
        let dest = target.join(rel);
        let data = embed::get(path.as_ref())
            .ok_or_else(|| HortoError::msg(format!("missing embed {path}")))?;
        fs::write_file(ctx, &dest, &data)?;
    }
    ctx.log(format!(
        "Copied docker source tree into {} (excluded stacks/)",
        target.display()
    ));
    Ok(())
}

fn merge_stack_prefix(ctx: &mut HostContext, target: &Path, stack_prefix: &str) -> Result<()> {
    let embed_prefix = format!("docker_source/{stack_prefix}");
    let mut count = 0u32;
    for path in Assets::iter() {
        let Some(rel) = path.strip_prefix(&embed_prefix) else {
            continue;
        };
        if rel.is_empty() {
            continue;
        }
        let dest = target.join(rel);
        let data = embed::get(path.as_ref())
            .ok_or_else(|| HortoError::msg(format!("missing embed {path}")))?;
        fs::write_file(ctx, &dest, &data)?;
        count += 1;
    }
    ctx.log(format!(
        "Merged {stack_prefix} ({count} files) into {}",
        target.display()
    ));
    Ok(())
}

fn load_render_vars(ctx: &HostContext) -> Result<BTreeMap<String, String>> {
    if ctx.paths.full_env_file().exists() {
        return envfile::load(&ctx.paths.full_env_file());
    }
    if ctx.paths.minimal_env_file().exists() {
        return envfile::load(&ctx.paths.minimal_env_file());
    }
    if ctx.is_dry_run() {
        let mut demo = BTreeMap::new();
        demo.insert("MY_HOSTNAME".into(), "horto-dryrun".into());
        demo.insert("WIFI_INTERFACE".into(), "wlan0".into());
        demo.insert("WIFI_SSID".into(), "Horto-IoT-LAN".into());
        demo.insert("WIFI_PASSPHRASE".into(), "".into());
        demo.insert("MY_URL".into(), "example.net".into());
        return Ok(demo);
    }
    Err(HortoError::msg(
        "no active setup file found; run s2 or m1 first",
    ))
}

fn copy_homepage_assets(ctx: &mut HostContext) -> Result<()> {
    let assets_target = ctx.paths.docker_assets();
    fs::ensure_dir(ctx, &assets_target)?;
    let mut copied = 0u32;
    for path in Assets::iter() {
        let Some(name) = path.strip_prefix("homepage_assets/") else {
            continue;
        };
        if name.is_empty() || !name.starts_with("homepage") {
            continue;
        }
        let data = embed::get(path.as_ref())
            .ok_or_else(|| HortoError::msg(format!("missing embed {path}")))?;
        let dest = assets_target.join(name);
        fs::write_file(ctx, &dest, &data)?;
        ctx.log(format!("Copied asset: {}", dest.display()));
        copied += 1;
    }
    if copied == 0 {
        ctx.log("No homepage* assets found; skipping asset copy.");
    }
    Ok(())
}

fn render_tree(
    ctx: &mut HostContext,
    root: &PathBuf,
    vars: &BTreeMap<String, String>,
) -> Result<()> {
    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        if !template::has_unreplaced_placeholders(&text) {
            continue;
        }
        let rendered = template::render(&text, vars);
        std::fs::write(path, rendered)?;
        ctx.log(format!("Rendered placeholders in: {}", path.display()));
    }
    Ok(())
}

fn download_piper_models(ctx: &mut HostContext) {
    let piper_dir = ctx.paths.docker.join("docker_repos/piper");
    if let Err(e) = std::fs::create_dir_all(&piper_dir) {
        ctx.log(format!("warning: cannot create piper dir: {e}"));
        return;
    }
    let url = "https://github.com/Hanzo-Huang/rk3576-home-assistant-voice/releases/download/models-v1/piper-rk3576-models.tar.gz";
    let archive = piper_dir.join("piper-rk3576-models.tar.gz");
    ctx.log("Downloading piper models...");
    let status = Command::new("wget")
        .args(["-O", archive.to_str().unwrap_or("/dev/null"), url])
        .current_dir(&piper_dir)
        .status();
    match status {
        Ok(s) if s.success() => {
            let stacks = ctx.paths.docker.join("stacks");
            let _ = Command::new("tar")
                .args([
                    "-xzf",
                    archive.to_str().unwrap_or(""),
                    "-C",
                    stacks.to_str().unwrap_or("/tmp"),
                ])
                .status();
            ctx.log("Piper models extracted.");
        }
        _ => {
            ctx.log(
                "warning: piper model download failed or wget missing; continuing without models",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_npu_type;

    #[test]
    fn npu_aliases_map_to_stack_folders() {
        assert_eq!(normalize_npu_type("rkRK3588"), "rk3588");
        assert_eq!(normalize_npu_type("rk3588"), "rk3588");
        assert_eq!(normalize_npu_type("none"), "none");
        assert_eq!(normalize_npu_type("no_wyoming"), "no_wyoming");
    }
}

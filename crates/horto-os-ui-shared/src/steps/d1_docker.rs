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
    use super::*;
    use crate::context::{ApplyMode, NonInteractivePrompts};
    use crate::kits::envfile;
    use crate::paths::HostPaths;
    use crate::pipeline::SetupKind;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    fn temp_paths(root: &std::path::Path) -> HostPaths {
        HostPaths {
            active_setup: root.join("active_setup"),
            backup: root.join("backup"),
            docker: root.join("docker"),
            etc: root.join("etc"),
            lease_file: root.join("leases"),
        }
    }

    #[test]
    fn npu_aliases_map_to_stack_folders() {
        assert_eq!(normalize_npu_type("rkRK3588"), "rk3588");
        assert_eq!(normalize_npu_type("rk3588"), "rk3588");
        assert_eq!(normalize_npu_type("RK3588"), "rk3588");
        assert_eq!(normalize_npu_type("rkRK3576"), "rk3576");
        assert_eq!(normalize_npu_type("RK3576"), "rk3576");
        assert_eq!(normalize_npu_type("rk3576"), "rk3576");
        assert_eq!(normalize_npu_type("none"), "none");
        assert_eq!(normalize_npu_type(""), "none");
        assert_eq!(normalize_npu_type("no_wyoming"), "no_wyoming");
        assert_eq!(normalize_npu_type("custom"), "custom");
    }

    #[test]
    fn trait_metadata_is_stable() {
        let step = D1Docker;
        assert_eq!(step.id(), "d1");
        assert_eq!(step.reference_script(), "d1_docker_init.sh");
        assert_eq!(step.step_version(), 2);
        assert!(step.depends_on().is_empty());
        assert!(!step.title().is_empty());
        assert!(!step.needs_reboot_after());
        assert!(!step.destructive());
    }

    #[test]
    fn is_done_detects_docker_artifacts() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        let ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        assert!(!D1Docker.is_done(&ctx));

        std::fs::create_dir_all(ctx.paths.docker.join("stacks")).unwrap();
        assert!(D1Docker.is_done(&ctx));
    }

    #[test]
    fn is_done_detects_dockge_compose() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        let ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        std::fs::create_dir_all(ctx.paths.docker.join("dockge")).unwrap();
        std::fs::write(
            ctx.paths.docker.join("dockge/compose.yaml"),
            b"services: {}\n",
        )
        .unwrap();
        assert!(D1Docker.is_done(&ctx));
    }

    #[test]
    fn plan_dry_run_lists_stack_merges() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        let planned = D1Docker.plan(&mut ctx).unwrap();
        assert!(planned.iter().any(|p| p.summary.contains("stacks/common")));
        assert!(planned
            .iter()
            .any(|p| p.summary.contains("extract embedded docker_source")));
        assert!(planned.iter().any(|p| p.summary.contains("piper")));
    }

    #[test]
    fn plan_skip_piper_when_flag_set() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        ctx.skip_piper = true;
        let planned = D1Docker.plan(&mut ctx).unwrap();
        assert!(planned
            .iter()
            .any(|p| p.summary.contains("skip piper model download")));
    }

    #[test]
    fn resolve_npu_type_reads_config_file() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        let mut map = BTreeMap::new();
        map.insert("NPU_TYPE".into(), "rk3588".into());
        envfile::write(&paths.os_configuration_file(), &map).unwrap();
        let ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        assert_eq!(resolve_npu_type(&ctx).unwrap(), "rk3588");
    }

    #[test]
    fn resolve_npu_type_rejects_unknown_stack() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        let mut map = BTreeMap::new();
        map.insert("NPU_TYPE".into(), "bogus".into());
        envfile::write(&paths.os_configuration_file(), &map).unwrap();
        let ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        let err = resolve_npu_type(&ctx).unwrap_err();
        assert!(err.to_string().contains("bogus"));
    }

    #[test]
    fn resolve_npu_type_dry_run_reads_embedded_template() {
        let tmp = TempDir::new().unwrap();
        let ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        // Embedded os-configuration.env ships NPU_TYPE=rkRK3588 => normalizes to rk3588.
        let npu = resolve_npu_type(&ctx).unwrap();
        assert_eq!(npu, "rk3588");
    }

    #[test]
    fn resolve_npu_type_apply_without_file_errors() {
        let tmp = TempDir::new().unwrap();
        let ctx =
            HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        let err = resolve_npu_type(&ctx).unwrap_err();
        assert!(err.to_string().contains("os-configuration"));
    }

    #[test]
    fn ensure_os_configuration_writes_template_when_missing() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        ensure_os_configuration(&mut ctx).unwrap();
        assert!(ctx.paths.os_configuration_file().is_file());
    }

    #[test]
    fn ensure_os_configuration_leaves_existing_file() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        std::fs::write(paths.os_configuration_file(), b"KEEP=me\n").unwrap();
        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        ensure_os_configuration(&mut ctx).unwrap();
        let body = std::fs::read_to_string(ctx.paths.os_configuration_file()).unwrap();
        assert!(body.contains("KEEP=me"));
    }

    #[test]
    fn copy_homepage_assets_creates_docker_assets_dir() {
        let tmp = TempDir::new().unwrap();
        let mut ctx =
            HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        copy_homepage_assets(&mut ctx).unwrap();
        assert!(ctx.paths.docker_assets().is_dir());
    }

    #[test]
    fn load_render_vars_prefers_full_env() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        let mut map = BTreeMap::new();
        map.insert("MY_HOSTNAME".into(), "prefer-full".into());
        envfile::write(&paths.full_env_file(), &map).unwrap();
        // Also drop a minimal env; full should win.
        let mut mmap = BTreeMap::new();
        mmap.insert("MY_HOSTNAME".into(), "prefer-minimal".into());
        envfile::write(&paths.minimal_env_file(), &mmap).unwrap();
        let ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        let vars = load_render_vars(&ctx).unwrap();
        assert_eq!(
            vars.get("MY_HOSTNAME").map(String::as_str),
            Some("prefer-full")
        );
    }

    #[test]
    fn load_render_vars_falls_back_to_minimal() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        std::fs::create_dir_all(&paths.active_setup).unwrap();
        let mut map = BTreeMap::new();
        map.insert("MY_HOSTNAME".into(), "mini-only".into());
        envfile::write(&paths.minimal_env_file(), &map).unwrap();
        let ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        let vars = load_render_vars(&ctx).unwrap();
        assert_eq!(
            vars.get("MY_HOSTNAME").map(String::as_str),
            Some("mini-only")
        );
    }

    #[test]
    fn load_render_vars_dry_run_returns_demo_vars() {
        let tmp = TempDir::new().unwrap();
        let ctx =
            HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        let vars = load_render_vars(&ctx).unwrap();
        assert!(vars.contains_key("MY_HOSTNAME"));
        assert!(vars.contains_key("WIFI_INTERFACE"));
    }

    #[test]
    fn load_render_vars_apply_without_files_errors() {
        let tmp = TempDir::new().unwrap();
        let ctx =
            HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(temp_paths(tmp.path()));
        let err = load_render_vars(&ctx).unwrap_err();
        assert!(err.to_string().contains("active setup"));
    }

    #[test]
    fn render_tree_replaces_placeholders_in_files() {
        let tmp = TempDir::new().unwrap();
        let paths = temp_paths(tmp.path());
        let root = tmp.path().join("tree");
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("hosts");
        std::fs::write(&target, b"host={{MY_HOSTNAME}}\n").unwrap();
        // Non-placeholder file should be untouched.
        let untouched = root.join("static");
        std::fs::write(&untouched, b"plain content\n").unwrap();

        let mut ctx = HostContext::new(ApplyMode::Apply, SetupKind::Full).with_paths(paths);
        let mut vars = BTreeMap::new();
        vars.insert("MY_HOSTNAME".into(), "cov-box".into());
        render_tree(&mut ctx, &root, &vars).unwrap();
        let rendered = std::fs::read_to_string(&target).unwrap();
        assert!(rendered.contains("cov-box"));
        assert_eq!(
            std::fs::read_to_string(&untouched).unwrap(),
            "plain content\n"
        );
    }

    #[test]
    fn apply_skips_piper_when_dry_run() {
        let tmp = TempDir::new().unwrap();
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full)
            .with_paths(temp_paths(tmp.path()))
            .with_prompts(Box::new(NonInteractivePrompts));
        D1Docker.apply(&mut ctx).unwrap();
        assert!(!ctx.planned.is_empty());
    }
}

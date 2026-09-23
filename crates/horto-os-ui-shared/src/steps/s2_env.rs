//! reference: horto-os/scripts/s2_init_env_vars.sh (+ `s2_init_env_vars_iot.sh`)
use crate::context::{HostContext, PlannedAction};
use crate::embed;
use crate::error::{HortoError, Result};
use crate::kits::{apt, envfile, fs};
use crate::step::Step;
use std::collections::BTreeMap;
use std::process::Command;

/// Collect and write OS / `IoT` env files (`s2`).
pub struct S2Env;

const OS_REQUIRED: &[&str] = &[
    "MY_HOSTNAME",
    "OS_TYPE",
    "NPU_TYPE",
    "INSTALL_TYPE",
    "IOT_LAN",
];
const IOT_ETH_REQUIRED: &[&str] = &["ETH_LAN", "ETH_IOT1", "WIFI_INTERFACE"];
const IOT_PACKAGES_BASE: &[&str] = &["dnsmasq", "iptables", "avahi-daemon"];
const IOT_PACKAGES_WIFI: &[&str] = &["hostapd"];

impl Step for S2Env {
    fn id(&self) -> &'static str {
        "s2"
    }
    fn title(&self) -> &'static str {
        "Configure environment variables"
    }
    fn reference_script(&self) -> &'static str {
        "s2_init_env_vars.sh"
    }
    fn step_version(&self) -> u32 {
        4
    }
    fn depends_on(&self) -> &'static [&'static str] {
        &["s1"]
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        let os_path = ctx.paths.os_configuration_file();
        let Ok(mut os_map) = envfile::load(&os_path) else {
            return false;
        };
        migrate_install_type_key(&mut os_map);
        if envfile::require_keys(&os_map, OS_REQUIRED).is_err() {
            return false;
        }
        if wants_iot_lan(&os_map) {
            let iot_path = ctx.paths.iot_lan_env_file();
            return envfile::load(&iot_path).is_ok_and(|m| iot_env_complete(&m));
        }
        true
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        ctx.plan_action(format!(
            "create/update {} from embedded config/os-configuration.env",
            ctx.paths.os_configuration_file().display()
        ));
        ctx.plan_action("prompt OS_TYPE, NPU_TYPE, INSTALL_TYPE, IOT_LAN, hostname, URL");
        ctx.plan_action(
            "if IOT_LAN=y: apt IoT packages + iot-lan_conf.env + ETH discovery (WiFi optional)",
        );
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        fs::ensure_dir(ctx, &ctx.paths.active_setup.clone())?;
        let os_active = ctx.paths.os_configuration_file();
        let mut os_map = load_or_embed(ctx, &os_active, "config/os-configuration.env")?;

        if ctx.is_dry_run() {
            ctx.plan_action(format!("write {}", os_active.display()));
            ctx.plan_action("optional IoT-LAN env + packages when IOT_LAN=y");
            return Ok(());
        }

        migrate_install_type_key(&mut os_map);
        prompt_os_conf(ctx, &mut os_map);
        migrate_install_type_key(&mut os_map);
        envfile::require_keys(&os_map, OS_REQUIRED)?;
        envfile::write(&os_active, &os_map)?;
        ctx.log(format!("Saved OS configuration to {}", os_active.display()));

        if wants_iot_lan(&os_map) {
            apply_iot_lan(ctx, &os_map)?;
        } else {
            ctx.log("setup without IOT_LAN detected");
        }
        Ok(())
    }
}

fn load_or_embed(
    ctx: &HostContext,
    active: &std::path::Path,
    embed_path: &str,
) -> Result<BTreeMap<String, String>> {
    if active.exists() && !ctx.is_dry_run() {
        return envfile::load(active);
    }
    let template =
        embed::get_str(embed_path).ok_or_else(|| HortoError::EmbedMissing(embed_path.into()))?;
    Ok(envfile::parse(&template))
}

fn prompt_os_conf(ctx: &mut HostContext, map: &mut BTreeMap<String, String>) {
    let hostname = ctx.prompt(
        "Device hostname",
        map.get("MY_HOSTNAME")
            .map_or("Horto-OS_xxx", String::as_str),
    );
    let os_type = ctx.prompt(
        "OS type (debian/armbian)",
        map.get("OS_TYPE").map_or("debian", String::as_str),
    );
    let npu = ctx.prompt(
        "NPU type (rk3588/rk3576/...)",
        map.get("NPU_TYPE").map_or("rk3588", String::as_str),
    );
    let ram = ctx.prompt(
        "RAM size label",
        map.get("RAM_SYZE").map_or("8gb", String::as_str),
    );
    let install = ctx.prompt(
        "Install type (home/satellite/hortex)",
        install_type_default(map),
    );
    let iot = ctx.prompt(
        "Enable IOT-LAN (y/n)",
        map.get("IOT_LAN").map_or("n", String::as_str),
    );
    let my_url = ctx.prompt(
        "Public URL / domain",
        map.get("MY_URL")
            .map_or("YourDomainName.net", String::as_str),
    );
    let cf = ctx.prompt(
        "Cloudflare token (optional)",
        map.get("MY_CLOUDFLARE_TOKEN").map_or("", String::as_str),
    );

    envfile::set_key(map, "MY_HOSTNAME", hostname);
    envfile::set_key(map, "OS_TYPE", os_type);
    envfile::set_key(map, "NPU_TYPE", npu);
    envfile::set_key(map, "RAM_SYZE", ram);
    envfile::set_key(map, "INSTALL_TYPE", install);
    map.remove("INSTALL_TYP");
    envfile::set_key(map, "IOT_LAN", normalize_yn(&iot));
    envfile::set_key(map, "MY_URL", my_url);
    envfile::set_key(map, "MY_CLOUDFLARE_TOKEN", cf);
}

/// Tip renamed `INSTALL_TYP` → `INSTALL_TYPE`; keep reading the old key once.
fn migrate_install_type_key(map: &mut BTreeMap<String, String>) {
    if map.contains_key("INSTALL_TYPE") {
        map.remove("INSTALL_TYP");
        return;
    }
    if let Some(legacy) = map.remove("INSTALL_TYP") {
        envfile::set_key(map, "INSTALL_TYPE", legacy);
    }
}

fn install_type_default(map: &BTreeMap<String, String>) -> &str {
    map.get("INSTALL_TYPE")
        .or_else(|| map.get("INSTALL_TYP"))
        .map_or("home", String::as_str)
}

fn apply_iot_lan(ctx: &mut HostContext, os_map: &BTreeMap<String, String>) -> Result<()> {
    ctx.log("IOT-LAN setup is next");

    let iot_active = ctx.paths.iot_lan_env_file();
    let mut map = load_or_embed(ctx, &iot_active, "config/iot-lan_conf.env")?;

    if let Some(hostname) = os_map.get("MY_HOSTNAME") {
        envfile::set_key(&mut map, "MY_HOSTNAME", hostname.clone());
    }

    let wifi_raw = ctx.prompt(
        "WiFi interface (none = Ethernet-only)",
        map.get("WIFI_INTERFACE").map_or("none", String::as_str),
    );
    let wifi_if = envfile::normalize_wifi_iface(&wifi_raw);
    envfile::set_key(&mut map, "WIFI_INTERFACE", wifi_if.clone());

    if envfile::wifi_iface_enabled(&wifi_if) {
        let wifi_ssid = ctx.prompt(
            "WiFi SSID",
            map.get("WIFI_SSID").map_or("Horto-IoT-LAN", String::as_str),
        );
        let wifi_pass = ctx.prompt(
            "WiFi passphrase",
            map.get("WIFI_PASSPHRASE").map_or("", String::as_str),
        );
        envfile::set_key(&mut map, "WIFI_SSID", wifi_ssid);
        envfile::set_key(&mut map, "WIFI_PASSPHRASE", wifi_pass);
    } else {
        ctx.log("WIFI_INTERFACE=none; Ethernet-only IoT-LAN (hostapd skipped)");
        envfile::set_key(&mut map, "WIFI_SSID", "");
        envfile::set_key(&mut map, "WIFI_PASSPHRASE", "");
    }

    install_iot_packages(ctx, envfile::wifi_iface_enabled(&wifi_if))?;
    discover_eth(&mut map, ctx);
    envfile::require_keys(&map, IOT_ETH_REQUIRED)?;
    if envfile::wifi_ap_enabled(&map) {
        envfile::require_keys(&map, &["WIFI_SSID"])?;
    }
    envfile::write(&iot_active, &map)?;
    ctx.log(format!(
        "Saved IoT-LAN variables to {}",
        iot_active.display()
    ));
    Ok(())
}

fn install_iot_packages(ctx: &mut HostContext, wifi: bool) -> Result<()> {
    ctx.log("Installing IoT LAN components...");
    if !crate::context::is_root() {
        ctx.log("Not root; skipping IoT apt install");
        return Ok(());
    }
    apt::apt_install(ctx, IOT_PACKAGES_BASE)?;
    if wifi {
        apt::apt_install(ctx, IOT_PACKAGES_WIFI)?;
        ctx.log("Base packages + hostapd for IOT-LAN installed.");
    } else {
        ctx.log("Base packages for Ethernet-only IOT-LAN installed (no hostapd).");
    }
    Ok(())
}

fn iot_env_complete(map: &BTreeMap<String, String>) -> bool {
    if envfile::require_keys(map, IOT_ETH_REQUIRED).is_err() {
        return false;
    }
    if envfile::wifi_ap_enabled(map) {
        return envfile::require_keys(map, &["WIFI_SSID"]).is_ok();
    }
    true
}

fn wants_iot_lan(map: &BTreeMap<String, String>) -> bool {
    map.get("IOT_LAN")
        .is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

fn normalize_yn(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "y" | "yes" => "y".into(),
        _ => "n".into(),
    }
}

fn discover_eth(map: &mut BTreeMap<String, String>, ctx: &mut HostContext) {
    assign_eth(map, ctx, &list_eth_ifaces());
}

fn assign_eth(
    map: &mut BTreeMap<String, String>,
    ctx: &mut HostContext,
    ifaces: &[(String, bool)],
) {
    let (eth0, eth1, eth2) = pick_eth(ifaces);
    envfile::set_key(map, "ETH_LAN", eth0.clone());
    envfile::set_key(map, "ETH_IOT1", eth1.clone());
    if let Some(e2) = eth2 {
        envfile::set_key(map, "ETH_IOT2", e2.clone());
        ctx.log(format!(
            "Discovered ETH_LAN={eth0}, ETH_IOT1={eth1}, ETH_IOT2={e2}"
        ));
    } else {
        ctx.log(format!(
            "Discovered ETH_LAN={eth0}, ETH_IOT1={eth1}, ETH_IOT2=not-set"
        ));
    }
}

fn list_eth_ifaces() -> Vec<(String, bool)> {
    let output = Command::new("ip").args(["-o", "link", "show"]).output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.split_once(": ").map(|(_, r)| r) else {
            continue;
        };
        let name = rest.split(':').next().unwrap_or("").trim();
        if !is_candidate_eth(name) {
            continue;
        }
        let lower_up = line.contains("LOWER_UP");
        out.push((name.to_string(), lower_up));
    }
    out
}

fn is_candidate_eth(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    if n == "lo"
        || n.starts_with("wlan")
        || n.starts_with("wlx")
        || n.starts_with("docker")
        || n.starts_with("br")
        || n.starts_with("veth")
        || n.starts_with("virbr")
    {
        return false;
    }
    n.starts_with("en") || n.starts_with("eth") || n == "wan" || n.starts_with("lan")
}

fn pick_eth(ifaces: &[(String, bool)]) -> (String, String, Option<String>) {
    let names: Vec<&str> = ifaces.iter().map(|(n, _)| n.as_str()).collect();
    if names.contains(&"wan") && names.iter().any(|n| n.starts_with("lan")) {
        let eth0 = "wan".to_string();
        let mut lans: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| n.starts_with("lan"))
            .collect();
        lans.sort_unstable();
        let eth1 = lans.first().unwrap_or(&"lan1").to_string();
        let eth2 = lans.get(1).map(|s| (*s).to_string());
        return (eth0, eth1, eth2);
    }

    let active: Vec<&str> = ifaces
        .iter()
        .filter(|(_, up)| *up)
        .map(|(n, _)| n.as_str())
        .collect();
    if !active.is_empty() {
        let eth0 = active[0].to_string();
        let rest: Vec<&str> = ifaces
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| *n != eth0)
            .collect();
        let eth1 = rest.first().unwrap_or(&"lan1").to_string();
        let eth2 = rest.get(1).map(|s| (*s).to_string());
        return (eth0, eth1, eth2);
    }
    let eth0 = names.first().unwrap_or(&"wan").to_string();
    let eth1 = names.get(1).unwrap_or(&"lan1").to_string();
    let eth2 = names.get(2).map(|s| (*s).to_string());
    (eth0, eth1, eth2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iot_lan_flag_parsing() {
        let mut m = BTreeMap::new();
        m.insert("IOT_LAN".into(), "y".into());
        assert!(wants_iot_lan(&m));
        m.insert("IOT_LAN".into(), "yes".into());
        assert!(wants_iot_lan(&m));
        m.insert("IOT_LAN".into(), "n".into());
        assert!(!wants_iot_lan(&m));
        assert_eq!(normalize_yn("YES"), "y");
        assert_eq!(normalize_yn("no"), "n");
    }

    #[test]
    fn iot_env_complete_allows_wifi_none() {
        let mut m = BTreeMap::new();
        m.insert("ETH_LAN".into(), "wan".into());
        m.insert("ETH_IOT1".into(), "lan1".into());
        m.insert("WIFI_INTERFACE".into(), "none".into());
        assert!(iot_env_complete(&m));

        m.insert("WIFI_INTERFACE".into(), "wlan0".into());
        assert!(!iot_env_complete(&m));
        m.insert("WIFI_SSID".into(), "Horto-IoT-LAN".into());
        assert!(iot_env_complete(&m));
    }

    #[test]
    fn candidate_eth_includes_horto_names() {
        assert!(is_candidate_eth("wan"));
        assert!(is_candidate_eth("lan1"));
        assert!(is_candidate_eth("lan2"));
        assert!(is_candidate_eth("eth0"));
        assert!(is_candidate_eth("enp1s0"));
        assert!(!is_candidate_eth("lo"));
        assert!(!is_candidate_eth("wlan0"));
        assert!(!is_candidate_eth("docker0"));
        assert!(!is_candidate_eth("br0"));
    }

    #[test]
    fn pick_eth_prefers_wan_and_lan_on_r6s() {
        let ifaces = vec![
            ("lan2".into(), true),
            ("wan".into(), true),
            ("lan1".into(), false),
        ];
        let (eth0, eth1, eth2) = pick_eth(&ifaces);
        assert_eq!(eth0, "wan");
        assert_eq!(eth1, "lan1");
        assert_eq!(eth2.as_deref(), Some("lan2"));
    }

    #[test]
    fn pick_eth_uses_first_active_when_no_wan_lan() {
        let ifaces = vec![
            ("enp1s0".into(), false),
            ("eth0".into(), true),
            ("eth1".into(), false),
        ];
        let (eth0, eth1, eth2) = pick_eth(&ifaces);
        assert_eq!(eth0, "eth0");
        assert_eq!(eth1, "enp1s0");
        assert_eq!(eth2.as_deref(), Some("eth1"));
    }

    #[test]
    fn pick_eth_falls_back_when_none_up() {
        let ifaces = vec![
            ("eth0".into(), false),
            ("eth1".into(), false),
            ("eth2".into(), false),
        ];
        let (eth0, eth1, eth2) = pick_eth(&ifaces);
        assert_eq!(eth0, "eth0");
        assert_eq!(eth1, "eth1");
        assert_eq!(eth2.as_deref(), Some("eth2"));
    }

    #[test]
    fn pick_eth_empty_defaults_to_wan_lan1() {
        let (eth0, eth1, eth2) = pick_eth(&[]);
        assert_eq!(eth0, "wan");
        assert_eq!(eth1, "lan1");
        assert!(eth2.is_none());
    }

    #[test]
    fn assign_eth_logs_iot2_when_present() {
        use crate::context::{ApplyMode, HostContext};
        use crate::pipeline::SetupKind;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let paths = crate::paths::HostPaths {
            active_setup: tmp.path().join("active"),
            backup: tmp.path().join("backup"),
            docker: tmp.path().join("docker"),
            etc: tmp.path().join("etc"),
            lease_file: tmp.path().join("leases"),
        };
        let mut ctx = HostContext::new(ApplyMode::DryRun, SetupKind::Full).with_paths(paths);
        let mut map = BTreeMap::new();
        let ifaces = vec![
            ("wan".into(), true),
            ("lan1".into(), true),
            ("lan2".into(), true),
        ];
        assign_eth(&mut map, &mut ctx, &ifaces);
        assert_eq!(map.get("ETH_LAN").map(String::as_str), Some("wan"));
        assert_eq!(map.get("ETH_IOT1").map(String::as_str), Some("lan1"));
        assert_eq!(map.get("ETH_IOT2").map(String::as_str), Some("lan2"));
        assert!(ctx.logs.iter().any(|l| l.contains("ETH_IOT2=lan2")));

        let mut map2 = BTreeMap::new();
        assign_eth(&mut map2, &mut ctx, &[("eth0".into(), false)]);
        assert_eq!(map2.get("ETH_LAN").map(String::as_str), Some("eth0"));
        assert!(!map2.contains_key("ETH_IOT2"));
        assert!(ctx.logs.iter().any(|l| l.contains("ETH_IOT2=not-set")));
    }

    #[test]
    fn iot_env_complete_false_without_eth_keys() {
        let mut m = BTreeMap::new();
        m.insert("WIFI_INTERFACE".into(), "none".into());
        assert!(!iot_env_complete(&m));
    }

    #[test]
    fn migrate_install_type_promotes_legacy_key() {
        let mut map = BTreeMap::new();
        map.insert("INSTALL_TYP".into(), "satellite".into());
        migrate_install_type_key(&mut map);
        assert_eq!(
            map.get("INSTALL_TYPE").map(String::as_str),
            Some("satellite")
        );
        assert!(!map.contains_key("INSTALL_TYP"));
    }

    #[test]
    fn migrate_install_type_drops_legacy_when_new_present() {
        let mut map = BTreeMap::new();
        map.insert("INSTALL_TYPE".into(), "home".into());
        map.insert("INSTALL_TYP".into(), "hortex".into());
        migrate_install_type_key(&mut map);
        assert_eq!(map.get("INSTALL_TYPE").map(String::as_str), Some("home"));
        assert!(!map.contains_key("INSTALL_TYP"));
    }

    #[test]
    fn install_type_default_reads_legacy_and_fallback() {
        let mut map = BTreeMap::new();
        assert_eq!(install_type_default(&map), "home");
        map.insert("INSTALL_TYP".into(), "hortex".into());
        assert_eq!(install_type_default(&map), "hortex");
        map.insert("INSTALL_TYPE".into(), "satellite".into());
        assert_eq!(install_type_default(&map), "satellite");
    }
}

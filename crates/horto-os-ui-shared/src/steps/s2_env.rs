//! reference: horto-os/scripts/s2_init_env_vars.sh (+ s2_helper_script.sh)
use crate::context::{HostContext, PlannedAction};
use crate::embed;
use crate::error::{HortoError, Result};
use crate::kits::{envfile, fs};
use crate::step::Step;
use std::collections::BTreeMap;
use std::process::Command;

pub struct S2Env;

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
        1
    }
    fn depends_on(&self) -> &'static [&'static str] {
        &["s1"]
    }
    fn is_done(&self, ctx: &HostContext) -> bool {
        let path = ctx.paths.full_env_file();
        if !path.exists() {
            return false;
        }
        envfile::load(&path)
            .ok()
            .and_then(|m| {
                envfile::require_keys(&m, &["MY_HOSTNAME", "WIFI_INTERFACE", "WIFI_SSID"]).ok()
            })
            .is_some()
    }
    fn plan(&self, ctx: &mut HostContext) -> Result<Vec<PlannedAction>> {
        let dest = ctx.paths.full_env_file();
        ctx.plan_action(format!(
            "create/update {} from embedded config/my_variables.env",
            dest.display()
        ));
        ctx.plan_action("prompt hostname, wifi iface/ssid/passphrase, URL");
        ctx.plan_action("discover ETH_LAN / ETH_IOT* via ip -o link");
        Ok(ctx.planned.clone())
    }
    fn apply(&self, ctx: &mut HostContext) -> Result<()> {
        fs::ensure_dir(ctx, &ctx.paths.active_setup.clone())?;
        let active = ctx.paths.full_env_file();
        let mut map = if active.exists() && !ctx.is_dry_run() {
            envfile::load(&active)?
        } else {
            let template = embed::get_str("config/my_variables.env")
                .ok_or_else(|| HortoError::EmbedMissing("config/my_variables.env".into()))?;
            envfile::parse(&template)
        };

        if ctx.is_dry_run() {
            ctx.plan_action(format!("write {}", active.display()));
            return Ok(());
        }

        let hostname = ctx.prompt(
            "Device hostname",
            map.get("MY_HOSTNAME")
                .map(String::as_str)
                .unwrap_or("Horto-OS_xxx"),
        );
        let wifi_if = ctx.prompt(
            "WiFi interface",
            map.get("WIFI_INTERFACE")
                .map(String::as_str)
                .unwrap_or("wlx0_xxxxx"),
        );
        let wifi_ssid = ctx.prompt(
            "WiFi SSID",
            map.get("WIFI_SSID")
                .map(String::as_str)
                .unwrap_or("Horto-IoT-LAN"),
        );
        let wifi_pass = ctx.prompt(
            "WiFi passphrase",
            map.get("WIFI_PASSPHRASE").map(String::as_str).unwrap_or(""),
        );
        let my_url = ctx.prompt(
            "Public URL / domain",
            map.get("MY_URL")
                .map(String::as_str)
                .unwrap_or("YourDomainName.net"),
        );

        envfile::set_key(&mut map, "MY_HOSTNAME", hostname);
        envfile::set_key(&mut map, "WIFI_INTERFACE", wifi_if);
        envfile::set_key(&mut map, "WIFI_SSID", wifi_ssid);
        envfile::set_key(&mut map, "WIFI_PASSPHRASE", wifi_pass);
        envfile::set_key(&mut map, "MY_URL", my_url);

        discover_eth(&mut map, ctx);

        envfile::require_keys(&map, &["MY_HOSTNAME", "WIFI_INTERFACE", "WIFI_SSID"])?;
        envfile::write(&active, &map)?;
        ctx.log(format!("Saved active variables to {}", active.display()));
        Ok(())
    }
}

fn discover_eth(map: &mut BTreeMap<String, String>, ctx: &mut HostContext) {
    let ifaces = list_en_ifaces();
    let (eth0, eth1, eth2) = pick_eth(&ifaces);
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

fn list_en_ifaces() -> Vec<(String, bool)> {
    let output = Command::new("ip").args(["-o", "link", "show"]).output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in text.lines() {
        // format: "2: enP3p49s0: <BROADCAST,...,LOWER_UP> ..."
        let Some(rest) = line.split_once(": ").map(|(_, r)| r) else {
            continue;
        };
        let name = rest.split(':').next().unwrap_or("").trim();
        if !name.starts_with("en") {
            continue;
        }
        let lower_up = line.contains("LOWER_UP");
        out.push((name.to_string(), lower_up));
    }
    out
}

fn pick_eth(ifaces: &[(String, bool)]) -> (String, String, Option<String>) {
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
    let names: Vec<&str> = ifaces.iter().map(|(n, _)| n.as_str()).collect();
    let eth0 = names.first().unwrap_or(&"wan").to_string();
    let eth1 = names.get(1).unwrap_or(&"lan1").to_string();
    let eth2 = names.get(2).map(|s| (*s).to_string());
    (eth0, eth1, eth2)
}

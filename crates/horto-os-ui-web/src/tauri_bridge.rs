//! Tauri IPC for the desktop webview.
//!
//! Always call `window.__TAURI_INTERNALS__.invoke` (Tauri 2). Do not use
//! `window.__TAURI__.core` — that path depends on `withGlobalTauri` and fragile
//! JS `Object` casts that fail in the wasm bindgen layer.

#![allow(clippy::future_not_send)]

use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;

/// True when running inside the Horto Tauri shell (not a plain browser Trunk build).
#[must_use]
pub fn is_desktop_shell() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    Reflect::get(&window, &"__TAURI_INTERNALS__".into())
        .ok()
        .is_some_and(|v| !v.is_undefined() && !v.is_null())
}

fn resolve_invoke() -> Result<(JsValue, Function), String> {
    let window = web_sys::window().ok_or_else(|| "no window".to_owned())?;
    let internals = Reflect::get(&window, &"__TAURI_INTERNALS__".into()).map_err(|_| {
        "Could not talk to the desktop shell (__TAURI_INTERNALS__ missing). Restart Horto."
            .to_owned()
    })?;
    if internals.is_undefined() || internals.is_null() {
        return Err(
            "Could not talk to the desktop shell (__TAURI_INTERNALS__ missing). Restart Horto."
                .into(),
        );
    }
    let invoke = Reflect::get(&internals, &"invoke".into()).map_err(|e| format!("{e:?}"))?;
    let invoke: Function = invoke
        .dyn_into()
        .map_err(|_| "Tauri invoke is not a function".to_owned())?;
    Ok((internals, invoke))
}

async fn invoke_cmd(cmd: &str, args: &JsValue) -> Result<JsValue, String> {
    let (this, invoke) = resolve_invoke()?;
    let promise = invoke
        .call2(&this, &cmd.into(), args)
        .map_err(|e| format!("invoke({cmd}) failed: {e:?}"))?;
    let promise: Promise = promise
        .dyn_into()
        .map_err(|_| format!("invoke({cmd}) did not return a Promise"))?;
    JsFuture::from(promise)
        .await
        .map_err(|e| format!("{cmd} error: {e:?}"))
}

/// Read the shared tip bearer via Desktop (same file as TUI / CLI).
///
/// Empty `Ok` means no usable tip yet.
pub async fn invoke_read_api_token() -> Result<String, String> {
    let value = invoke_cmd("read_api_token", &Object::new().into()).await?;
    if value.is_null() || value.is_undefined() {
        return Ok(String::new());
    }
    Ok(value
        .as_string()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_default())
}

/// Write the shared tip bearer via Desktop (same file as TUI / CLI).
pub async fn invoke_write_api_token(token: &str) -> Result<(), String> {
    let args = Object::new();
    Reflect::set(&args, &"token".into(), &token.into()).map_err(|e| format!("{e:?}"))?;
    invoke_cmd("write_api_token", &args.into()).await?;
    Ok(())
}

/// Pull box bearer over SSH into the tip file (same as TUI save after install).
pub async fn invoke_sync_api_token_from_box(host: &str) -> Result<String, String> {
    let args = Object::new();
    Reflect::set(&args, &"host".into(), &host.into()).map_err(|e| format!("{e:?}"))?;
    let value = invoke_cmd("sync_api_token_from_box", &args.into()).await?;
    value
        .as_string()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "sync_api_token_from_box returned empty token".into())
}

/// One row from Desktop `list_known_remote_hosts_cmd`.
#[derive(Debug, Clone)]
pub struct KnownHostUi {
    /// OpenSSH / Status API target name.
    pub name: String,
}

/// List LAN + local host names (Desktop only).
pub async fn invoke_list_known_remote_hosts() -> Result<Vec<KnownHostUi>, String> {
    let value = invoke_cmd("list_known_remote_hosts_cmd", &JsValue::NULL).await?;
    parse_known_hosts_value(&value)
}

fn parse_known_hosts_value(value: &JsValue) -> Result<Vec<KnownHostUi>, String> {
    let arr = js_sys::Array::from(value);
    let mut out = Vec::with_capacity(arr.length() as usize);
    for i in 0..arr.length() {
        let item = arr.get(i);
        let name = Reflect::get(&item, &"name".into())
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_default();
        if name.trim().is_empty() {
            continue;
        }
        out.push(KnownHostUi { name });
    }
    Ok(out)
}

/// List GitHub Release tags for the install picker (Desktop only).
pub async fn invoke_list_release_tags() -> Result<Vec<String>, String> {
    let value = invoke_cmd("list_release_tags_cmd", &JsValue::NULL).await?;
    let arr = js_sys::Array::from(&value);
    let mut out = Vec::with_capacity(arr.length() as usize);
    for i in 0..arr.length() {
        if let Some(tag) = arr.get(i).as_string() {
            let tag = tag.trim().to_owned();
            if !tag.is_empty() {
                out.push(tag);
            }
        }
    }
    Ok(out)
}

/// Probe SSH / CLI / API / MCP surfaces on `host`.
pub async fn invoke_remote_surfaces_probe(host: &str) -> Result<JsValue, String> {
    let args = Object::new();
    Reflect::set(&args, &"host".into(), &host.into()).map_err(|e| format!("{e:?}"))?;
    invoke_cmd("remote_surfaces_probe", &args.into()).await
}

/// Run remote setup; returns `{ log, apiToken? }`.
pub async fn invoke_remote_setup_cmd(payload: &Object) -> Result<JsValue, String> {
    let args = Object::new();
    Reflect::set(&args, &"args".into(), payload).map_err(|e| format!("{e:?}"))?;
    invoke_cmd("remote_setup", &args.into()).await
}

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

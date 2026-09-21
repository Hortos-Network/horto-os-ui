//! Thin Tauri invoke helpers shared by the desktop webview.
//!
//! Uses `window.__TAURI__.core.invoke` when `withGlobalTauri` is on, otherwise
//! falls back to `window.__TAURI_INTERNALS__.invoke` (always present in the
//! Tauri 2 webview).

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
    has_js_object(&window, "__TAURI_INTERNALS__") || has_js_object(&window, "__TAURI__")
}

fn has_js_object(window: &web_sys::Window, key: &str) -> bool {
    Reflect::get(window, &key.into())
        .ok()
        .is_some_and(|v| !v.is_undefined() && !v.is_null())
}

enum InvokeTarget {
    /// `__TAURI__.core.invoke(cmd, args)` — `this` is core.
    Core { core: Object, invoke: Function },
    /// `__TAURI_INTERNALS__.invoke(cmd, args)` — `this` is internals.
    Internals { internals: Object, invoke: Function },
}

fn resolve_invoke() -> Result<InvokeTarget, String> {
    let window = web_sys::window().ok_or_else(|| "no window".to_owned())?;

    if let Ok(tauri) = Reflect::get(&window, &"__TAURI__".into()) {
        if !tauri.is_undefined() && !tauri.is_null() {
            let core = Reflect::get(&tauri, &"core".into()).map_err(|e| format!("{e:?}"))?;
            let core_obj: Object = core
                .clone()
                .dyn_into()
                .map_err(|_| "Tauri core missing".to_owned())?;
            let invoke = Reflect::get(&core, &"invoke".into()).map_err(|e| format!("{e:?}"))?;
            let invoke: Function = invoke
                .dyn_into()
                .map_err(|_| "invoke is not a function".to_owned())?;
            return Ok(InvokeTarget::Core {
                core: core_obj,
                invoke,
            });
        }
    }

    let internals = Reflect::get(&window, &"__TAURI_INTERNALS__".into())
        .map_err(|_| "Could not talk to the desktop shell. Restart Horto.".to_owned())?;
    if internals.is_undefined() || internals.is_null() {
        return Err("Could not talk to the desktop shell. Restart Horto.".into());
    }
    let internals_obj: Object = internals
        .clone()
        .dyn_into()
        .map_err(|_| "Tauri internals missing".to_owned())?;
    let invoke = Reflect::get(&internals, &"invoke".into()).map_err(|e| format!("{e:?}"))?;
    let invoke: Function = invoke
        .dyn_into()
        .map_err(|_| "invoke is not a function".to_owned())?;
    Ok(InvokeTarget::Internals {
        internals: internals_obj,
        invoke,
    })
}

async fn invoke_cmd(cmd: &str, args: &JsValue) -> Result<JsValue, String> {
    let target = resolve_invoke()?;
    let promise = match target {
        InvokeTarget::Core { core, invoke } => invoke
            .call2(&core, &cmd.into(), args)
            .map_err(|e| format!("invoke failed: {e:?}"))?,
        InvokeTarget::Internals { internals, invoke } => invoke
            .call2(&internals, &cmd.into(), args)
            .map_err(|e| format!("invoke failed: {e:?}"))?,
    };
    let promise: Promise = promise
        .dyn_into()
        .map_err(|_| "invoke did not return a Promise".to_owned())?;
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

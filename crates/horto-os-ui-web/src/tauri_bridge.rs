//! Thin `__TAURI__.core.invoke` helpers shared by the desktop webview.

#![allow(clippy::future_not_send)]

use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

fn tauri_invoke() -> Result<(js_sys::Object, Function), String> {
    let window = web_sys::window().ok_or_else(|| "no window".to_owned())?;
    let tauri = Reflect::get(&window, &"__TAURI__".into())
        .map_err(|_| "Could not talk to the desktop shell. Restart Horto.".to_owned())?;
    if tauri.is_undefined() || tauri.is_null() {
        return Err("Could not talk to the desktop shell. Restart Horto.".into());
    }
    let core = Reflect::get(&tauri, &"core".into()).map_err(|e| format!("{e:?}"))?;
    let core_obj: Object = core
        .clone()
        .dyn_into()
        .map_err(|_| "Tauri core missing".to_owned())?;
    let invoke = Reflect::get(&core, &"invoke".into()).map_err(|e| format!("{e:?}"))?;
    let invoke: Function = invoke
        .dyn_into()
        .map_err(|_| "invoke is not a function".to_owned())?;
    Ok((core_obj, invoke))
}

/// Read the shared tip bearer via Desktop (same file as TUI / CLI).
pub async fn invoke_read_api_token() -> Result<Option<String>, String> {
    let (core, invoke) = tauri_invoke()?;
    let promise = invoke
        .call2(&core, &"read_api_token".into(), &Object::new())
        .map_err(|e| format!("invoke failed: {e:?}"))?;
    let promise: Promise = promise
        .dyn_into()
        .map_err(|_| "invoke did not return a Promise".to_owned())?;
    let value = JsFuture::from(promise)
        .await
        .map_err(|e| format!("read_api_token error: {e:?}"))?;
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    Ok(value.as_string().filter(|s| !s.trim().is_empty()))
}

/// Write the shared tip bearer via Desktop (same file as TUI / CLI).
pub async fn invoke_write_api_token(token: &str) -> Result<(), String> {
    let (core, invoke) = tauri_invoke()?;
    let args = Object::new();
    Reflect::set(&args, &"token".into(), &token.into()).map_err(|e| format!("{e:?}"))?;
    let promise = invoke
        .call2(&core, &"write_api_token".into(), &args)
        .map_err(|e| format!("invoke failed: {e:?}"))?;
    let promise: Promise = promise
        .dyn_into()
        .map_err(|_| "invoke did not return a Promise".to_owned())?;
    JsFuture::from(promise)
        .await
        .map_err(|e| format!("write_api_token error: {e:?}"))?;
    Ok(())
}

/// Pull box bearer over SSH into the tip file (same as TUI save after install).
pub async fn invoke_sync_api_token_from_box(host: &str) -> Result<String, String> {
    let (core, invoke) = tauri_invoke()?;
    let args = Object::new();
    Reflect::set(&args, &"host".into(), &host.into()).map_err(|e| format!("{e:?}"))?;
    let promise = invoke
        .call2(&core, &"sync_api_token_from_box".into(), &args)
        .map_err(|e| format!("invoke failed: {e:?}"))?;
    let promise: Promise = promise
        .dyn_into()
        .map_err(|_| "invoke did not return a Promise".to_owned())?;
    let value = JsFuture::from(promise)
        .await
        .map_err(|e| format!("sync_api_token_from_box error: {e:?}"))?;
    value
        .as_string()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "sync_api_token_from_box returned empty token".into())
}

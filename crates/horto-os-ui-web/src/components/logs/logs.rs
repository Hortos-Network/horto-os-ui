use leptos::prelude::*;
use rangular_aot::HostCell;
use rangular_host::{Host, HostError, Value};
use wasm_bindgen::JsCast;

use crate::tauri_bridge::{
    invoke_append_app_log, invoke_clear_app_logs, invoke_list_app_logs, is_desktop_shell,
};

use super::state::{LogRow, LogsState};

include!(concat!(env!("OUT_DIR"), "/rangular/logs_view.rs"));

/// One-shot hydrate + live `horto-log` listener.
pub fn boot_logs(state: LogsState) {
    if state.is_booted() {
        return;
    }
    state.mark_booted();
    if !is_desktop_shell() {
        return;
    }
    leptos::task::spawn_local(async move {
        if let Ok(rows) = invoke_list_app_logs().await {
            state.replace_all(
                rows.into_iter()
                    .map(|r| LogRow {
                        seq: r.seq,
                        ts_ms: r.ts_ms,
                        level: r.level,
                        target: r.target,
                        message: r.message,
                    })
                    .collect(),
            );
        }
    });
    attach_log_listener(state);
}

#[component]
pub fn LogsPanel(state: LogsState) -> impl IntoView {
    Effect::new(move |_| {
        let _ = state.entries.get();
        let follow = state.follow.get();
        if follow {
            scroll_stream_to_end();
        }
    });

    logs_view(HostCell::new(LogsHost { state }))
}

struct LogsHost {
    state: LogsState,
}

impl Host for LogsHost {
    fn get(&self, name: &str) -> Option<Value> {
        let filter = self.state.level_filter.get();
        let search = self.state.search.get();
        let entries = self.state.entries.get();
        let visible = visible_rows(&entries, &filter, &search);
        let desktop = is_desktop_shell();
        let empty = visible.is_empty();
        match name {
            "seqs" => Some(Value::List(
                visible
                    .iter()
                    .map(|r| Value::Str(r.seq.to_string()))
                    .collect(),
            )),
            "search" => Some(Value::Str(self.state.search.get())),
            "follow" => Some(Value::Bool(self.state.follow.get())),
            "filterAll" => Some(Value::Bool(filter.eq_ignore_ascii_case("ALL"))),
            "filterError" => Some(Value::Bool(filter.eq_ignore_ascii_case("ERROR"))),
            "filterWarn" => Some(Value::Bool(filter.eq_ignore_ascii_case("WARN"))),
            "filterInfo" => Some(Value::Bool(filter.eq_ignore_ascii_case("INFO"))),
            "filterDebug" => Some(Value::Bool(
                filter.eq_ignore_ascii_case("DEBUG") || filter.eq_ignore_ascii_case("TRACE"),
            )),
            "isEmpty" => Some(Value::Bool(empty)),
            "showDesktopEmpty" => Some(Value::Bool(desktop && empty)),
            "showBrowserEmpty" => Some(Value::Bool(!desktop)),
            "canCopy" => Some(Value::Bool(desktop && !visible.is_empty())),
            "canClear" => Some(Value::Bool(desktop && !entries.is_empty())),
            "metaLabel" => Some(Value::Str(format!(
                "{} shown · {} total",
                visible.len(),
                entries.len()
            ))),
            _ => None,
        }
    }

    fn set(&mut self, name: &str, value: Value) -> Result<(), HostError> {
        match name {
            "search" => {
                if let Some(s) = value.as_str() {
                    self.state.search.set(s.to_owned());
                }
            }
            "follow" => {
                if let Some(b) = value.as_bool() {
                    self.state.follow.set(b);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn call(&mut self, name: &str, args: &[Value]) -> Result<Value, HostError> {
        match name {
            "setFilterAll" => self.state.level_filter.set("ALL".into()),
            "setFilterError" => self.state.level_filter.set("ERROR".into()),
            "setFilterWarn" => self.state.level_filter.set("WARN".into()),
            "setFilterInfo" => self.state.level_filter.set("INFO".into()),
            "setFilterDebug" => self.state.level_filter.set("DEBUG".into()),
            "clearLogs" => {
                let state = self.state;
                leptos::task::spawn_local(async move {
                    let _ = invoke_clear_app_logs().await;
                    state.entries.set(Vec::new());
                });
            }
            "copyLogs" => {
                let filter = self.state.level_filter.get();
                let search = self.state.search.get();
                let entries = self.state.entries.get();
                let text = format_visible(&entries, &filter, &search);
                copy_text(&text);
            }
            "onScroll" => {
                if self.state.follow.get() && !stream_near_bottom() {
                    self.state.follow.set(false);
                }
            }
            "timeAt" | "isoAt" | "levelAt" | "targetAt" | "messageAt" | "isError" | "isWarn"
            | "isInfo" | "isDebug" => {
                return Ok(row_field(self, name, args));
            }
            _ => {}
        }
        Ok(Value::Unit)
    }
}

fn row_field(host: &LogsHost, name: &str, args: &[Value]) -> Value {
    let Some(i) = arg_index(args) else {
        return Value::Unit;
    };
    let filter = host.state.level_filter.get();
    let search = host.state.search.get();
    let entries = host.state.entries.get();
    let visible = visible_rows(&entries, &filter, &search);
    let Some(row) = visible.get(i) else {
        return Value::Unit;
    };
    match name {
        "timeAt" => Value::Str(format_time(row.ts_ms)),
        "isoAt" => Value::Str(format_iso(row.ts_ms)),
        "levelAt" => Value::Str(row.level.clone()),
        "targetAt" => Value::Str(short_target(&row.target)),
        "messageAt" => Value::Str(row.message.clone()),
        "isError" => Value::Bool(row.level.eq_ignore_ascii_case("ERROR")),
        "isWarn" => Value::Bool(row.level.eq_ignore_ascii_case("WARN")),
        "isInfo" => Value::Bool(row.level.eq_ignore_ascii_case("INFO")),
        "isDebug" => Value::Bool(
            row.level.eq_ignore_ascii_case("DEBUG") || row.level.eq_ignore_ascii_case("TRACE"),
        ),
        _ => Value::Unit,
    }
}

fn visible_rows<'a>(rows: &'a [LogRow], filter: &str, search: &str) -> Vec<&'a LogRow> {
    let search = search.trim().to_ascii_lowercase();
    rows.iter()
        .filter(|r| level_matches(filter, &r.level))
        .filter(|r| {
            search.is_empty()
                || r.message.to_ascii_lowercase().contains(&search)
                || r.target.to_ascii_lowercase().contains(&search)
                || r.level.to_ascii_lowercase().contains(&search)
                || r.seq.to_string().contains(&search)
        })
        .collect()
}

fn level_matches(filter: &str, level: &str) -> bool {
    if filter.eq_ignore_ascii_case("ALL") {
        return true;
    }
    if filter.eq_ignore_ascii_case("DEBUG") {
        return level.eq_ignore_ascii_case("DEBUG") || level.eq_ignore_ascii_case("TRACE");
    }
    level.eq_ignore_ascii_case(filter)
}

fn format_visible(rows: &[LogRow], filter: &str, search: &str) -> String {
    visible_rows(rows, filter, search)
        .into_iter()
        .map(|r| {
            format!(
                "#{}\t{}\t{}\t{}\t{}",
                r.seq,
                format_time(r.ts_ms),
                r.level,
                r.target,
                r.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_time(ts_ms: u64) -> String {
    let secs = (ts_ms / 1000) % 86_400;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    let ms = ts_ms % 1000;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

fn format_iso(ts_ms: u64) -> String {
    // UTC ISO-ish for datetime attr; display uses local-less clock digits above.
    let secs = ts_ms / 1000;
    let ms = ts_ms % 1000;
    format!("{secs}.{ms:03}")
}

fn short_target(target: &str) -> String {
    target
        .rsplit("::")
        .next()
        .unwrap_or(target)
        .chars()
        .take(28)
        .collect()
}

fn arg_index(args: &[Value]) -> Option<usize> {
    match args.first()? {
        Value::Num(n) if n.is_finite() && *n >= 0.0 =>
        {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Some(*n as usize)
        }
        _ => None,
    }
}

fn scroll_stream_to_end() {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    if let Ok(Some(el)) = document.query_selector(".logs__stream") {
        if let Some(el) = el.dyn_ref::<web_sys::HtmlElement>() {
            el.set_scroll_top(el.scroll_height());
        }
    }
}

fn stream_near_bottom() -> bool {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return true;
    };
    let Ok(Some(el)) = document.query_selector(".logs__stream") else {
        return true;
    };
    let Some(el) = el.dyn_ref::<web_sys::HtmlElement>() else {
        return true;
    };
    let remaining = el.scroll_height() - el.scroll_top() - el.client_height();
    remaining < 48
}

fn copy_text(text: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let clipboard = window.navigator().clipboard();
    let _ = clipboard.write_text(text);
}

fn attach_log_listener(state: LogsState) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let closure = wasm_bindgen::closure::Closure::wrap(Box::new(move |event: web_sys::Event| {
        let Some(custom) = event.dyn_ref::<web_sys::CustomEvent>() else {
            return;
        };
        if let Some(row) = parse_log_detail(&custom.detail()) {
            state.push_row(row);
        }
    }) as Box<dyn FnMut(_)>);
    let _ = window.add_event_listener_with_callback("horto-log", closure.as_ref().unchecked_ref());
    closure.forget();
}

fn parse_log_detail(detail: &wasm_bindgen::JsValue) -> Option<LogRow> {
    use js_sys::Reflect;
    let seq = Reflect::get(detail, &"seq".into())
        .ok()?
        .as_f64()
        .map(|n| n as u64)?;
    let ts_ms = Reflect::get(detail, &"ts_ms".into())
        .ok()
        .and_then(|v| v.as_f64())
        .map_or(0, |n| n as u64);
    let level = Reflect::get(detail, &"level".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| "INFO".into());
    let target = Reflect::get(detail, &"target".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    let message = Reflect::get(detail, &"message".into())
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_default();
    if message.is_empty() {
        return None;
    }
    Some(LogRow {
        seq,
        ts_ms,
        level,
        target,
        message,
    })
}

/// Append a UI / action line into the single Desktop log ring (no separate streams).
pub fn app_log(level: &str, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    if !is_desktop_shell() {
        return;
    }
    let level = level.to_owned();
    let text = text.to_owned();
    leptos::task::spawn_local(async move {
        let _ = invoke_append_app_log(&level, "horto", &text).await;
    });
}

/// Append at INFO.
pub fn app_log_info(text: &str) {
    app_log("INFO", text);
}

/// Append at WARN / ERROR when a call fails.
pub fn app_log_error(text: &str) {
    app_log("ERROR", text);
}

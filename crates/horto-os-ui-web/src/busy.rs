//! Shared async-busy helpers for Desktop / web action buttons.
//!
//! Pair with `.horto-btn` / `.horto-btn--busy` in `style/main.css`.

use leptos::prelude::*;
use std::future::Future;

/// Run `job` while `busy` is true. No-ops when already busy (untracked read).
pub fn spawn_busy(busy: RwSignal<bool>, job: impl Future<Output = ()> + 'static) {
    if busy.get_untracked() {
        return;
    }
    spawn_busy_force(busy, job);
}

/// Always start `job` and clear `busy` when it finishes (even if already busy).
///
/// Use for Reload-style actions so a stuck busy flag cannot kill the button.
pub fn spawn_busy_force(busy: RwSignal<bool>, job: impl Future<Output = ()> + 'static) {
    busy.set(true);
    leptos::task::spawn_local(async move {
        job.await;
        busy.set(false);
    });
}

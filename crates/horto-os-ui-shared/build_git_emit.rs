//! Emit `GIT_COMMIT_HASH` for Horto UI crates (include from each `build.rs`).

use std::process::Command;

/// Print `cargo:rustc-env=GIT_COMMIT_HASH=…` (short SHA, optional `-dirty`).
pub fn emit_git_commit_hash() {
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into());

    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| !s.trim().is_empty());

    let label = if dirty && git_hash != "unknown" {
        format!("{git_hash}-dirty")
    } else {
        git_hash
    };

    println!("cargo:rustc-env=GIT_COMMIT_HASH={label}");

    // Workspace root is two levels up from each `crates/<pkg>` (horto-os-ui/.git).
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
}

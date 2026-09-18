//! Emit git commit for `horto_os_ui_shared::build_info`.

#[path = "build_git_emit.rs"]
mod build_git_emit;

fn main() {
    build_git_emit::emit_git_commit_hash();
}

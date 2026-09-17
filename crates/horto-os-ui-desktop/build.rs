#[path = "../horto-os-ui-shared/build_git_emit.rs"]
mod build_git_emit;

fn main() {
    build_git_emit::emit_git_commit_hash();
    tauri_build::build();
}

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[path = "../horto-os-ui-shared/build_git_emit.rs"]
mod build_git_emit;

fn main() {
    build_git_emit::emit_git_commit_hash();

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let components = manifest.join("src/components");
    let css_out = manifest.join("style/components.generated.css");
    let out_dir = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("rangular");
    std::fs::create_dir_all(&out_dir).expect("create rangular OUT_DIR");

    println!("cargo:rerun-if-changed=src/components");

    let panels = [
        ("top_bar", "top_bar_view"),
        ("connection", "connection_view"),
        ("box_status", "box_status_view"),
        ("services", "services_view"),
        ("containers", "containers_view"),
    ];

    let mut css = String::from(
        "/* Generated from Horto rangular panel SCSS - do not edit. */\n@layer components {\n",
    );
    for (dir, fn_name) in panels {
        compile_panel(&components, &out_dir, &mut css, dir, fn_name);
    }
    css.push_str("}\n");

    std::fs::write(&css_out, css).unwrap_or_else(|err| {
        panic!("write {}: {err}", css_out.display());
    });
}

fn compile_panel(components: &Path, out_dir: &Path, css: &mut String, dir: &str, fn_name: &str) {
    let panel_dir = components.join(dir);
    append_scss(css, dir, &panel_dir.join(format!("{dir}.scss")));

    let html_path = panel_dir.join(format!("{dir}.html"));
    let html = std::fs::read_to_string(&html_path).unwrap_or_else(|err| {
        panic!("read {}: {err}", html_path.display());
    });
    let source = format!("src/components/{dir}/{dir}.html");
    let aot = rangular_aot::compile_named(&html, &source, fn_name);
    assert!(aot.ok(), "{dir}.html: {:?}", aot.issues);
    let rs_path = out_dir.join(format!("{fn_name}.rs"));
    std::fs::write(&rs_path, &aot.code).unwrap_or_else(|err| {
        panic!("write {}: {err}", rs_path.display());
    });
}

fn append_scss(css: &mut String, label: &str, scss_path: &Path) {
    let scss = std::fs::read_to_string(scss_path).unwrap_or_else(|err| {
        panic!("read {}: {err}", scss_path.display());
    });
    let result = rangular_css::compile_scss(&scss);
    assert!(result.ok(), "{label}.scss: {:?}", result.issues);
    let _ = write!(css, "\n/* --- {label} --- */\n");
    css.push_str(&result.css);
    css.push('\n');
}

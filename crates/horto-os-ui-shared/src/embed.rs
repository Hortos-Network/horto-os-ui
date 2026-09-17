use rust_embed::Embed;
use std::borrow::Cow;

/// Embedded setup templates and docker source tree.
#[derive(Embed)]
#[folder = "../../assets/"]
pub struct Assets;

pub fn get(path: &str) -> Option<Cow<'static, [u8]>> {
    Assets::get(path).map(|f| f.data)
}

pub fn get_str(path: &str) -> Option<String> {
    get(path).and_then(|d| String::from_utf8(d.into_owned()).ok())
}

/// List files under an embedded prefix (e.g. `config/` or `docker_source/`).
pub fn list_prefix(prefix: &str) -> Vec<String> {
    Assets::iter()
        .filter(|p| p.starts_with(prefix))
        .map(|p| p.to_string())
        .collect()
}

/// Top-level entries under `config/` (file or directory names as in the embed).
pub fn config_top_entries() -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for path in Assets::iter() {
        let Some(rest) = path.strip_prefix("config/") else {
            continue;
        };
        let name = rest.split('/').next().unwrap_or(rest);
        if !name.is_empty() {
            names.insert(name.to_string());
        }
    }
    names.into_iter().collect()
}

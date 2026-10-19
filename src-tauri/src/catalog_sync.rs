//! Which catalog is in use: the one built into RecompHUB, or a newer revision published in
//! the project's repository (as Ports Launcher refreshes its ports.json). A downloaded
//! catalog is kept in RecompHUB's data folder and used only when it validates and its
//! revision is higher than the built-in one.

use std::path::Path;

use crate::catalog;

/// The catalog in the project's repository. While the repository is private this address
/// answers 404 and RecompHUB keeps the built-in catalog.
const REMOTE_URL: &str = "https://raw.githubusercontent.com/genriquesento-ops/recomphub/main/src-tauri/catalog.json";
const CACHE_FILE: &str = "catalog.json";
const MAX_BYTES: usize = 2 * 1024 * 1024;

/// At start: the built-in catalog, or the cached download when it is newer and valid.
pub fn load_startup(data_dir: &Path) {
    let (builtin_rev, builtin, _) = catalog::parse(catalog::BUILTIN).expect("catálogo embutido inválido");
    let cached = std::fs::read_to_string(data_dir.join(CACHE_FILE)).ok().and_then(|t| catalog::parse(&t).ok());
    match cached {
        Some((rev, games, _)) if rev > builtin_rev && !games.is_empty() => catalog::install(rev, games),
        _ => catalog::install(builtin_rev, builtin),
    }
}

/// Fetches the repository's catalog; Ok(true) when a newer revision was installed.
pub async fn refresh(data_dir: &Path) -> Result<bool, String> {
    let resp = crate::github::client().get(REMOTE_URL).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Ok(false);
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if text.len() > MAX_BYTES {
        return Err("catálogo grande demais".into());
    }
    let (rev, games, _) = catalog::parse(&text)?;
    if rev <= catalog::revision() || games.is_empty() {
        return Ok(false);
    }
    let _ = std::fs::write(data_dir.join(CACHE_FILE), &text);
    catalog::install(rev, games);
    Ok(true)
}

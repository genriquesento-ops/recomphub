//! Downloading a port from its official release and unpacking it.

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};

use crate::catalog::GameDef;
use crate::github::{self, Release};

#[derive(Serialize, Clone)]
pub struct Progress {
    pub id: String,
    /// "download", "verify", "extract"
    pub stage: String,
    pub done: u64,
    pub total: u64,
}

fn emit(app: &AppHandle, id: &str, stage: &str, done: u64, total: u64) {
    let _ = app.emit("install-progress", Progress { id: id.into(), stage: stage.into(), done, total });
}

/// Downloads `release` into `target`, keeping the player's files listed in `keep_on_update`.
pub async fn install(app: &AppHandle, game: &GameDef, release: &Release, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| format!("Não consegui criar a pasta {}: {e}", target.display()))?;
    check_space(target, release.asset_size)?;
    let zip_path = target.with_file_name(format!("{}.download.zip", game.id));
    let result = download_and_extract(app, game, release, target, &zip_path).await;
    let _ = std::fs::remove_file(&zip_path);
    result
}

async fn download_and_extract(
    app: &AppHandle,
    game: &GameDef,
    release: &Release,
    target: &Path,
    zip_path: &Path,
) -> Result<(), String> {
    let client = github::client();
    let resp = client
        .get(&release.asset_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("O download falhou: {e}"))?;
    let total = resp.content_length().unwrap_or(release.asset_size);
    let mut file = std::fs::File::create(zip_path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    let mut last_emit = 0u64;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("O download foi interrompido: {e}"))?;
        file.write_all(&chunk).map_err(|e| e.to_string())?;
        hasher.update(&chunk);
        done += chunk.len() as u64;
        if done - last_emit > 512 * 1024 || done == total {
            emit(app, &game.id, "download", done, total);
            last_emit = done;
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    if let Some(url) = &release.sha256_url {
        emit(app, &game.id, "verify", 0, 0);
        let expected = client
            .get(url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("Não consegui baixar a soma de verificação: {e}"))?
            .text()
            .await
            .map_err(|e| e.to_string())?;
        let expected = expected.split_whitespace().next().unwrap_or("").to_lowercase();
        let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if expected != actual {
            return Err("O arquivo baixado está corrompido (a soma de verificação não confere). Tente de novo.".into());
        }
    }

    let (app2, zip_path, target) = (app.clone(), zip_path.to_path_buf(), target.to_path_buf());
    let keep = game.keep_on_update.clone();
    let id = game.id.clone();
    tauri::async_runtime::spawn_blocking(move || extract(&app2, &id, &zip_path, &target, &keep))
        .await
        .map_err(|e| e.to_string())?
}

fn extract(app: &AppHandle, id: &str, zip_path: &Path, target: &Path, keep: &[String]) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("O arquivo baixado não é um zip válido: {e}"))?;

    // Some releases put everything inside one folder: unpack its contents instead.
    let names: Vec<PathBuf> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().and_then(|f| f.enclosed_name()))
        .collect();
    let strip = common_root(&names);

    let total = archive.len() as u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = entry.enclosed_name() else { continue };
        let rel = match &strip {
            Some(root) => match name.strip_prefix(root) {
                Ok(r) if !r.as_os_str().is_empty() => r.to_path_buf(),
                _ => continue,
            },
            None => name,
        };
        let out = target.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        let rel_text = rel.to_string_lossy().replace('\\', "/");
        let kept = keep.iter().any(|k| if k.ends_with('/') { rel_text.starts_with(k.as_str()) } else { rel_text == *k });
        if kept && out.exists() {
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut f = std::fs::File::create(&out)
            .map_err(|e| format!("Não consegui gravar {} (o jogo está aberto?): {e}", out.display()))?;
        std::io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
        if i % 16 == 0 {
            emit(app, id, "extract", i as u64, total);
        }
    }
    emit(app, id, "extract", total, total);
    Ok(())
}

/// The download plus the unpacked port need about three times the zip's size.
fn check_space(target: &Path, zip_size: u64) -> Result<(), String> {
    let needed = zip_size.saturating_mul(3);
    let target = target.to_string_lossy().to_lowercase();
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let disk = disks
        .list()
        .iter()
        .filter(|d| target.starts_with(&d.mount_point().to_string_lossy().to_lowercase()))
        .max_by_key(|d| d.mount_point().as_os_str().len());
    match disk {
        Some(d) if d.available_space() < needed => {
            let gb = |b: u64| format!("{:.1} GB", b as f64 / 1_073_741_824.0);
            Err(format!(
                "Falta espaço no disco {}: são necessários {} e há {} livres.",
                d.mount_point().display(),
                gb(needed),
                gb(d.available_space())
            ))
        }
        _ => Ok(()),
    }
}

/// The single top-level folder every entry is inside, if there is one.
fn common_root(names: &[PathBuf]) -> Option<PathBuf> {
    let first = names.first()?.components().next()?;
    let root = PathBuf::from(first.as_os_str());
    let all_inside = names.iter().all(|n| {
        let mut c = n.components();
        c.next().map(|x| x.as_os_str() == first.as_os_str()).unwrap_or(false)
    });
    // A lone file at the top is not a folder to strip.
    let has_children = names.iter().any(|n| n.components().count() > 1);
    (all_inside && has_children).then_some(root)
}

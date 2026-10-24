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
    install_release(app, &game.id, release, target, &game.keep_on_update).await
}

/// What a release file is: an archive to unpack, or the program itself.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Format {
    Zip,
    SevenZ,
    /// A portable program: the download is the game's .exe.
    Exe,
}

pub fn format_of(name_or_url: &str) -> Format {
    let n = name_or_url.split(['?', '#']).next().unwrap_or("").to_lowercase();
    if n.ends_with(".7z") {
        Format::SevenZ
    } else if n.ends_with(".exe") {
        Format::Exe
    } else {
        Format::Zip
    }
}

/// The file name at the end of a download address ("%20" back to a space), when it is a
/// plain name.
pub fn file_name_of(url: &str) -> Option<String> {
    let last = url.rsplit('/').next()?.split(['?', '#']).next()?;
    let name = last.replace("%20", " ");
    let ok = !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', ':', '%', '*', '?', '"', '<', '>', '|']);
    ok.then_some(name)
}

/// The same for any release (community ports): `id` names the progress events and the
/// temporary download, `keep` lists the player's files an update must not replace.
pub async fn install_release(
    app: &AppHandle,
    id: &str,
    release: &Release,
    target: &Path,
    keep: &[String],
) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| format!("Não consegui criar a pasta {}: {e}", target.display()))?;
    check_space(target, release.asset_size)?;
    let format = format_of(&release.asset_url);
    let ext = match format {
        Format::Zip => "zip",
        Format::SevenZ => "7z",
        Format::Exe => "exe",
    };
    let zip_path = target.with_file_name(format!("{id}.download.{ext}"));
    let result = download_and_extract(app, id, release, target, &zip_path, keep).await;
    let _ = std::fs::remove_file(&zip_path);
    result
}

async fn download_and_extract(
    app: &AppHandle,
    id: &str,
    release: &Release,
    target: &Path,
    zip_path: &Path,
    keep: &[String],
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
            emit(app, id, "download", done, total);
            last_emit = done;
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    if let Some(url) = &release.sha256_url {
        emit(app, id, "verify", 0, 0);
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
    let keep = keep.to_vec();
    let id = id.to_string();
    let url = release.asset_url.clone();
    tauri::async_runtime::spawn_blocking(move || match format_of(&url) {
        Format::Zip => extract(&app2, &id, &zip_path, &target, &keep),
        Format::SevenZ => extract_7z(&app2, &id, &zip_path, &target, &keep),
        Format::Exe => {
            let name = file_name_of(&url).ok_or("O nome do programa baixado não é válido.")?;
            std::fs::copy(&zip_path, target.join(&name))
                .map(|_| ())
                .map_err(|e| crate::play::antivirus_error(&e).unwrap_or_else(|| format!("Não consegui gravar {name}: {e}")))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The player's files an update must not replace (Windows paths ignore case).
fn is_kept(rel: &Path, keep: &[String]) -> bool {
    let rel_text = rel.to_string_lossy().replace('\\', "/").to_lowercase();
    keep.iter()
        .map(|k| k.to_lowercase())
        .any(|k| if k.ends_with('/') { rel_text.starts_with(&k) } else { rel_text == k })
}

/// A name inside an archive that stays inside the folder it is unpacked to.
pub fn safe_entry(name: &str) -> bool {
    let n = name.replace('\\', "/");
    // Windows device names (CON, NUL, COM1...) and names ending in a dot or space are not files.
    let reserved = |c: &str| {
        let stem = c.split('.').next().unwrap_or("").to_ascii_lowercase();
        matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
            || ((stem.starts_with("com") || stem.starts_with("lpt")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit())
    };
    !n.is_empty()
        && !n.starts_with('/')
        && !n.contains(':')
        && n.split('/').filter(|c| !c.is_empty()).all(|c| c != ".." && !c.ends_with('.') && !c.ends_with(' ') && !reserved(c))
}

/// A .7z release: unpacked to a folder beside the port first (every name checked, the
/// archive refused when one would land outside), then placed like a .zip.
fn extract_7z(app: &AppHandle, id: &str, archive: &Path, target: &Path, keep: &[String]) -> Result<(), String> {
    let staging = target.with_file_name(format!("{id}.extraindo"));
    let _ = std::fs::remove_dir_all(&staging);
    emit(app, id, "extract", 0, 0);
    let unpacked = sevenz_rust::decompress_file_with_extract_fn(archive, &staging, |entry, reader, dest| {
        if !safe_entry(entry.name()) {
            return Err(sevenz_rust::Error::other("caminho perigoso dentro do pacote"));
        }
        sevenz_rust::default_entry_extract_fn(entry, reader, dest)
    });
    let result = unpacked
        .map_err(|e| format!("O arquivo .7z baixado não pôde ser aberto ({e})."))
        .and_then(|_| place(app, id, &staging, target, keep));
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// Copies an unpacked release into the port's folder: a single top folder is left out and the
/// player's kept files are not replaced.
fn place(app: &AppHandle, id: &str, staging: &Path, target: &Path, keep: &[String]) -> Result<(), String> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut stack = vec![staging.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| e.to_string())?.flatten() {
            let p = e.path();
            // The entry itself, not where a link would lead: links are left out.
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(staging) {
                files.push(rel.to_path_buf());
            }
        }
    }
    let strip = common_root(&files);
    let total = files.len() as u64;
    for (i, rel) in files.iter().enumerate() {
        let inner = match &strip {
            Some(root) => match rel.strip_prefix(root) {
                Ok(r) if !r.as_os_str().is_empty() => r.to_path_buf(),
                _ => continue,
            },
            None => rel.clone(),
        };
        let out = target.join(&inner);
        if is_kept(&inner, keep) && out.exists() {
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(staging.join(rel), &out).map_err(|e| {
            crate::precheck::write_error(&e, &out)
        })?;
        if i % 16 == 0 {
            emit(app, id, "extract", i as u64, total);
        }
    }
    emit(app, id, "extract", total, total);
    Ok(())
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
        // Windows paths ignore case: "Saves/" is the same folder as "saves/".
        if is_kept(&rel, keep) && out.exists() {
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut f = std::fs::File::create(&out).map_err(|e| {
            crate::precheck::write_error(&e, &out)
        })?;
        std::io::copy(&mut entry, &mut f).map_err(|e| crate::play::antivirus_error(&e).unwrap_or_else(|| e.to_string()))?;
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

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn release_files() {
        assert_eq!(format_of("https://github.com/a/b/releases/download/v1/Game-windows.7z"), Format::SevenZ);
        assert_eq!(format_of("Decomp.Buddy.0.30.5.Portable.exe"), Format::Exe);
        assert_eq!(format_of("game.zip"), Format::Zip);
        assert_eq!(file_name_of("https://github.com/a/b/releases/download/v1/My%20Game.exe").as_deref(), Some("My Game.exe"));
        assert_eq!(file_name_of("https://x/y/..%2F..%2Fevil.exe"), None);
        assert!(safe_entry("Game/bin/game.exe"));
        assert!(!safe_entry("../../evil.dll"));
        assert!(!safe_entry("Game\\..\\..\\evil.dll"));
        assert!(!safe_entry("C:/Windows/evil.dll"));
        assert!(!safe_entry("/etc/x"));
        assert!(!safe_entry("Game/CON"));
        assert!(!safe_entry("Game/nul.txt"));
        assert!(!safe_entry("Game/COM1.log"));
        assert!(!safe_entry("Game/evil. "));
        assert!(safe_entry("Game/console.dll"));
        assert_eq!(format_of("https://x/game.7z?raw=1"), Format::SevenZ);
        assert!(is_kept(Path::new("Saves\\slot1.sav"), &["saves/".to_string()]));
    }
}

#[cfg(test)]
mod tests {
    use super::common_root;
    use std::path::PathBuf;

    fn paths(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn single_top_folder_is_stripped() {
        let wrapped = paths(&["gow3-windows/", "gow3-windows/run.py", "gow3-windows/bin/x.dll"]);
        assert_eq!(common_root(&wrapped), Some(PathBuf::from("gow3-windows")));
        let flat = paths(&["BanjoRecompiled.exe", "SDL2.dll", "assets/Banjo.svg"]);
        assert_eq!(common_root(&flat), None);
        assert_eq!(common_root(&paths(&["only.exe"])), None);
        assert_eq!(common_root(&[]), None);
    }
}

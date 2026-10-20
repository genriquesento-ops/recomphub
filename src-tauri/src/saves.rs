//! The player's saves, as Ports Launcher keeps them:
//! - uninstalling a port moves the files the player owns out of its folder ("pendentes")
//!   and the next install of that port moves them back;
//! - "Backup dos saves" writes every port's saves into one dated .zip.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Files above this size are left out of backups (a shader cache is not a save).
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)? {
            let e = e?;
            if e.file_type()?.is_symlink() {
                continue;
            }
            copy_tree(&e.path(), &dst.join(e.file_name()))?;
        }
    } else {
        if let Some(p) = dst.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::copy(src, dst)?;
    }
    Ok(())
}

/// Moves a file or folder; across drives it copies and then deletes.
pub(crate) fn move_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    if let Some(p) = dst.parent() {
        std::fs::create_dir_all(p)?;
    }
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    copy_tree(src, dst)?;
    if src.is_dir() {
        std::fs::remove_dir_all(src)
    } else {
        std::fs::remove_file(src)
    }
}

/// Relative paths inside `install_dir` that belong to the player: the catalog's
/// keep_on_update list and the save folder when it lives inside the port's folder.
pub fn owned_paths(install_dir: &Path, keep: &[String], save_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut rel: Vec<PathBuf> = keep.iter().map(|k| PathBuf::from(k.trim_end_matches('/'))).collect();
    if let Some(r) = save_dir.and_then(|s| s.strip_prefix(install_dir).ok()) {
        if !r.as_os_str().is_empty() {
            rel.push(r.to_path_buf());
        }
    }
    // A path inside another one on the list is already carried by it.
    let mut out: Vec<PathBuf> = Vec::new();
    for r in rel {
        if !out.iter().any(|o| r.starts_with(o)) {
            out.retain(|o| !o.starts_with(&r));
            out.push(r);
        }
    }
    out.into_iter().filter(|r| install_dir.join(r).exists()).collect()
}

/// Before an uninstall: moves the player's paths to `pending/<id>`. Returns what was kept.
pub fn preserve(install_dir: &Path, owned: &[PathBuf], pending: &Path) -> Result<Vec<String>, String> {
    let mut kept = Vec::new();
    for rel in owned {
        let dst = pending.join(rel);
        if dst.exists() {
            let _ = if dst.is_dir() { std::fs::remove_dir_all(&dst) } else { std::fs::remove_file(&dst) };
        }
        move_tree(&install_dir.join(rel), &dst)
            .map_err(|e| format!("Não consegui guardar {} antes de desinstalar: {e}", rel.display()))?;
        kept.push(rel.display().to_string());
    }
    Ok(kept)
}

/// After an install: puts pending files back (the player's copy wins over the release's).
pub fn restore(pending: &Path, install_dir: &Path) -> Result<bool, String> {
    if !pending.is_dir() {
        return Ok(false);
    }
    copy_tree(pending, install_dir).map_err(|e| format!("Não consegui devolver os saves guardados: {e}"))?;
    std::fs::remove_dir_all(pending).map_err(|e| e.to_string())?;
    Ok(true)
}

fn add_dir(zip: &mut zip::ZipWriter<std::fs::File>, dir: &Path, prefix: &str, files: &mut usize) -> Result<(), String> {
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for e in entries.flatten() {
        let Ok(kind) = e.file_type() else { continue };
        let name = format!("{prefix}/{}", e.file_name().to_string_lossy());
        if kind.is_dir() {
            add_dir(zip, &e.path(), &name, files)?;
        } else if kind.is_file() && e.metadata().map(|m| m.len() <= MAX_FILE_BYTES).unwrap_or(false) {
            let mut data = Vec::new();
            std::fs::File::open(e.path()).and_then(|mut f| f.read_to_end(&mut data)).map_err(|e| e.to_string())?;
            zip.start_file(name, options).map_err(|e| e.to_string())?;
            zip.write_all(&data).map_err(|e| e.to_string())?;
            *files += 1;
        }
    }
    Ok(())
}

/// Writes `<folder>/<date>.zip` with one folder per game. Running it again the same day
/// replaces that day's file, so each date is one clean snapshot.
pub fn backup(items: &[(String, PathBuf)], folder: &Path, date: &str) -> Result<(PathBuf, usize), String> {
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let out = folder.join(format!("{date}.zip"));
    let tmp = folder.join(format!("{date}.zip.tmp"));
    let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let mut files = 0;
    for (name, dir) in items {
        add_dir(&mut zip, dir, name, &mut files)?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
    Ok((out, files))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("recomphub-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn saves_survive_uninstall_and_come_back() {
        let root = temp("saves");
        let install = root.join("port");
        let pending = root.join("pending");
        std::fs::create_dir_all(install.join("user/savedata")).unwrap();
        std::fs::write(install.join("user/savedata/slot1.bin"), b"save").unwrap();
        std::fs::write(install.join("gow3.ini"), b"fps=120").unwrap();
        std::fs::write(install.join("game.exe"), b"exe").unwrap();

        let keep = vec!["gow3.ini".to_string(), "user/".to_string(), "portable.txt".to_string()];
        let owned = owned_paths(&install, &keep, Some(&install.join("user/savedata")));
        assert_eq!(owned.len(), 2, "{owned:?}");
        preserve(&install, &owned, &pending).unwrap();
        std::fs::remove_dir_all(&install).unwrap();

        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("gow3.ini"), b"fps=60").unwrap();
        assert!(restore(&pending, &install).unwrap());
        assert_eq!(std::fs::read(install.join("user/savedata/slot1.bin")).unwrap(), b"save");
        assert_eq!(std::fs::read(install.join("gow3.ini")).unwrap(), b"fps=120");
        assert!(!pending.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn backup_zips_each_game() {
        let root = temp("backup");
        std::fs::create_dir_all(root.join("a/sub")).unwrap();
        std::fs::write(root.join("a/sub/s.sav"), b"x").unwrap();
        let (zip_path, files) = backup(&[("gow3".into(), root.join("a"))], &root.join("out"), "2026-10-19").unwrap();
        assert_eq!(files, 1);
        let mut z = zip::ZipArchive::new(std::fs::File::open(zip_path).unwrap()).unwrap();
        assert!(z.by_name("gow3/sub/s.sav").is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }
}

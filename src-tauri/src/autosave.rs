//! Automatic save backups, as Ludusavi's: one .zip per backup in
//! `Backups de saves/auto/<id>/`, made when the game closes (or when the player asks),
//! skipped when nothing changed since the last one, and trimmed to the newest N per game.
//! A locked backup (`<name>.lock` beside it) never counts toward that limit.
//!
//! Each .zip carries `recomphub-backup.json` saying which game it is, where every folder
//! came from and a fingerprint of the files, so a backup can be checked before it is restored.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MANIFEST: &str = "recomphub-backup.json";
/// Files above this size are left out (a shader cache is not a save).
const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;

/// One folder of the player's saves; `label` is its folder inside the .zip.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Source {
    pub label: String,
    pub path: String,
    #[serde(default)]
    pub files: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Manifest {
    pub format: u32,
    pub game: String,
    pub created: u64,
    /// "ao fechar o jogo", "manual" or "antes de restaurar".
    pub reason: String,
    pub sources: Vec<Source>,
    pub fingerprint: String,
    pub bytes: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct BackupInfo {
    pub file: String,
    pub created: u64,
    pub reason: String,
    pub files: usize,
    pub bytes: u64,
    pub locked: bool,
}

/// Every file under each source, as (zip name, path), sorted so the fingerprint is stable.
fn collect(sources: &[(String, PathBuf)]) -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let Ok(kind) = e.file_type() else { continue };
            let name = format!("{prefix}/{}", e.file_name().to_string_lossy());
            if kind.is_dir() {
                walk(&e.path(), &name, out);
            } else if kind.is_file() && e.metadata().map(|m| m.len() <= MAX_FILE_BYTES).unwrap_or(false) {
                out.push((name, e.path()));
            }
        }
    }
    let mut out = Vec::new();
    for (label, dir) in sources {
        walk(dir, label, &mut out);
    }
    out.sort();
    out
}

/// Newest modification time of the saves, for "último save" and the save watcher.
pub fn newest_change(sources: &[(String, PathBuf)]) -> Option<u64> {
    collect(sources)
        .iter()
        .filter_map(|(_, p)| std::fs::metadata(p).ok()?.modified().ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .max()
}

fn game_dir(root: &Path, id: &str) -> PathBuf {
    root.join(id)
}

fn read_manifest(zip_path: &Path) -> Option<Manifest> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(zip_path).ok()?).ok()?;
    let mut text = String::new();
    z.by_name(MANIFEST).ok()?.read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}

/// The game's backups, newest first.
pub fn list(root: &Path, id: &str) -> Vec<BackupInfo> {
    let dir = game_dir(root, id);
    let Ok(entries) = std::fs::read_dir(&dir) else { return vec![] };
    let mut out: Vec<BackupInfo> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "zip").unwrap_or(false))
        .filter_map(|p| {
            let m = read_manifest(&p)?;
            let file = p.file_name()?.to_string_lossy().to_string();
            Some(BackupInfo {
                locked: dir.join(format!("{file}.lock")).is_file(),
                file,
                created: m.created,
                reason: m.reason,
                files: m.sources.iter().map(|s| s.files).sum(),
                bytes: m.bytes,
            })
        })
        .collect();
    out.sort_by(|a, b| b.created.cmp(&a.created).then(b.file.cmp(&a.file)));
    out
}

/// "2026-10-20_13-05-09" (UTC), so names sort by time.
fn stamp(unix: u64) -> String {
    let s = unix % 86_400;
    format!("{}_{:02}-{:02}-{:02}", crate::library::date(unix), s / 3600, (s % 3600) / 60, s % 60)
}

pub enum Made {
    Created(BackupInfo),
    /// The saves are the same as in the newest backup.
    Unchanged,
    /// No save files yet.
    Empty,
}

/// Backs up `sources` unless they are identical to the newest backup, then keeps only the
/// newest `keep` unlocked backups.
pub fn make(root: &Path, id: &str, sources: &[(String, PathBuf)], reason: &str, keep: usize, now: u64) -> Result<Made, String> {
    let files = collect(sources);
    if files.is_empty() {
        return Ok(Made::Empty);
    }
    let mut hasher = Sha256::new();
    let mut data: Vec<(String, Vec<u8>)> = Vec::with_capacity(files.len());
    let mut bytes = 0u64;
    for (name, path) in &files {
        let mut buf = Vec::new();
        std::fs::File::open(path)
            .and_then(|mut f| f.read_to_end(&mut buf))
            .map_err(|e| format!("Não consegui ler {}: {e}", path.display()))?;
        hasher.update(name.as_bytes());
        hasher.update((buf.len() as u64).to_le_bytes());
        hasher.update(&buf);
        bytes += buf.len() as u64;
        data.push((name.clone(), buf));
    }
    let fingerprint = format!("{:x}", hasher.finalize());

    let dir = game_dir(root, id);
    if let Some(newest) = list(root, id).first() {
        if read_manifest(&dir.join(&newest.file)).map(|m| m.fingerprint == fingerprint).unwrap_or(false) {
            return Ok(Made::Unchanged);
        }
    }

    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut name = format!("{}.zip", stamp(now));
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("{}-{n}.zip", stamp(now));
        n += 1;
    }
    let manifest = Manifest {
        format: 1,
        game: id.to_string(),
        created: now,
        reason: reason.to_string(),
        sources: sources
            .iter()
            .map(|(label, path)| Source {
                label: label.clone(),
                path: path.display().to_string(),
                files: files.iter().filter(|(n, _)| n.starts_with(&format!("{label}/"))).count(),
            })
            .collect(),
        fingerprint,
        bytes,
    };
    let tmp = dir.join(format!("{name}.tmp"));
    let write = || -> Result<(), String> {
        let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(MANIFEST, options).map_err(|e| e.to_string())?;
        zip.write_all(serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?.as_bytes())
            .map_err(|e| e.to_string())?;
        for (name, buf) in &data {
            zip.start_file(name.as_str(), options).map_err(|e| e.to_string())?;
            zip.write_all(buf).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dir.join(&name)).map_err(|e| e.to_string())
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Não consegui criar o backup: {e}"));
    }
    trim(root, id, keep);
    let info = list(root, id).into_iter().find(|b| b.file == name).ok_or("O backup sumiu depois de criado.")?;
    Ok(Made::Created(info))
}

/// Deletes the oldest unlocked backups past `keep`.
pub fn trim(root: &Path, id: &str, keep: usize) {
    let dir = game_dir(root, id);
    for old in list(root, id).into_iter().filter(|b| !b.locked).skip(keep.max(1)) {
        let _ = std::fs::remove_file(dir.join(&old.file));
    }
}

fn valid_name(file: &str) -> bool {
    file.ends_with(".zip") && !file.contains(['/', '\\']) && !file.contains("..")
}

pub fn set_locked(root: &Path, id: &str, file: &str, locked: bool) -> Result<(), String> {
    let dir = game_dir(root, id);
    if !valid_name(file) || !dir.join(file).is_file() {
        return Err("Backup não encontrado.".into());
    }
    let lock = dir.join(format!("{file}.lock"));
    if locked {
        std::fs::write(lock, b"").map_err(|e| e.to_string())
    } else {
        let _ = std::fs::remove_file(lock);
        Ok(())
    }
}

/// Puts a backup back. Every file is read and checked against the backup's own list first;
/// each save folder is then swapped whole (new one written beside it, old one removed only
/// after the swap), so a failure never leaves a half-restored folder.
pub fn restore(root: &Path, id: &str, file: &str, targets: &[(String, PathBuf)]) -> Result<usize, String> {
    if !valid_name(file) {
        return Err("Backup não encontrado.".into());
    }
    let path = game_dir(root, id).join(file);
    let manifest = read_manifest(&path).ok_or("Este backup está danificado ou não é do RecompHUB.")?;
    if manifest.game != id {
        return Err("Este backup é de outro jogo.".into());
    }
    let mut z = zip::ZipArchive::new(std::fs::File::open(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    // label -> files
    let mut by_label: Vec<(String, Vec<(PathBuf, Vec<u8>)>)> = Vec::new();
    for i in 0..z.len() {
        let mut entry = z.by_index(i).map_err(|e| e.to_string())?;
        if entry.name() == MANIFEST || entry.is_dir() {
            continue;
        }
        let Some(rel) = entry.enclosed_name() else { return Err("O backup tem um caminho inválido.".into()) };
        let mut parts = rel.components();
        let label = parts.next().map(|c| c.as_os_str().to_string_lossy().to_string()).unwrap_or_default();
        let inner: PathBuf = parts.collect();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| format!("O backup está danificado: {e}"))?;
        match by_label.iter_mut().find(|(l, _)| *l == label) {
            Some((_, v)) => v.push((inner, buf)),
            None => by_label.push((label, vec![(inner, buf)])),
        }
    }
    for s in &manifest.sources {
        let found = by_label.iter().find(|(l, _)| *l == s.label).map(|(_, v)| v.len()).unwrap_or(0);
        if found != s.files {
            return Err(format!("O backup está incompleto: faltam arquivos de \"{}\".", s.label));
        }
    }
    let mut restored = 0;
    for (label, items) in by_label {
        let Some((_, target)) = targets.iter().find(|(l, _)| *l == label) else { continue };
        let parent = target.parent().ok_or("Pasta de saves inválida.")?;
        let name = target.file_name().ok_or("Pasta de saves inválida.")?.to_string_lossy().to_string();
        let fresh = parent.join(format!("{name}.recomphub-restaurando"));
        let old = parent.join(format!("{name}.recomphub-anterior"));
        let _ = std::fs::remove_dir_all(&fresh);
        let _ = std::fs::remove_dir_all(&old);
        for (inner, buf) in &items {
            let dst = fresh.join(inner);
            if let Some(p) = dst.parent() {
                std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
            }
            std::fs::write(&dst, buf).map_err(|e| format!("Não consegui escrever {}: {e}", dst.display()))?;
        }
        if target.exists() {
            std::fs::rename(target, &old).map_err(|e| format!("A pasta de saves está em uso? {e}"))?;
        }
        if let Err(e) = std::fs::rename(&fresh, target) {
            let _ = std::fs::rename(&old, target);
            return Err(format!("Não consegui colocar o backup no lugar: {e}"));
        }
        let _ = std::fs::remove_dir_all(&old);
        restored += items.len();
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("recomphub-auto-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn unchanged_saves_are_not_backed_up_twice_and_old_ones_are_trimmed() {
        let root = temp("trim");
        let saves = root.join("saves");
        std::fs::create_dir_all(&saves).unwrap();
        let src = vec![("saves".to_string(), saves.clone())];
        assert!(matches!(make(&root.join("b"), "banjo", &src, "manual", 2, 100).unwrap(), Made::Empty));

        std::fs::write(saves.join("bk.bin"), b"1").unwrap();
        assert!(matches!(make(&root.join("b"), "banjo", &src, "manual", 2, 100).unwrap(), Made::Created(_)));
        assert!(matches!(make(&root.join("b"), "banjo", &src, "manual", 2, 200).unwrap(), Made::Unchanged));

        let first = list(&root.join("b"), "banjo")[0].file.clone();
        set_locked(&root.join("b"), "banjo", &first, true).unwrap();
        for (i, t) in [(2u8, 300u64), (3, 400), (4, 500)] {
            std::fs::write(saves.join("bk.bin"), [i]).unwrap();
            make(&root.join("b"), "banjo", &src, "ao fechar o jogo", 2, t).unwrap();
        }
        let all = list(&root.join("b"), "banjo");
        assert_eq!(all.len(), 3, "2 recentes + 1 travado");
        assert!(all.iter().any(|b| b.file == first && b.locked));
        assert_eq!(all[0].created, 500);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_swaps_the_whole_folder_and_refuses_other_games() {
        let root = temp("restore");
        let saves = root.join("saves");
        std::fs::create_dir_all(saves.join("sub")).unwrap();
        std::fs::write(saves.join("sub/a.sav"), b"old").unwrap();
        let src = vec![("saves".to_string(), saves.clone())];
        let Made::Created(b) = make(&root.join("b"), "mm", &src, "manual", 5, 100).unwrap() else { panic!() };

        std::fs::write(saves.join("sub/a.sav"), b"new").unwrap();
        std::fs::write(saves.join("extra.sav"), b"x").unwrap();
        assert_eq!(restore(&root.join("b"), "mm", &b.file, &src).unwrap(), 1);
        assert_eq!(std::fs::read(saves.join("sub/a.sav")).unwrap(), b"old");
        assert!(!saves.join("extra.sav").exists(), "a pasta volta inteira ao estado do backup");

        std::fs::create_dir_all(root.join("b/banjo")).unwrap();
        std::fs::copy(root.join("b/mm").join(&b.file), root.join("b/banjo").join(&b.file)).unwrap();
        assert!(restore(&root.join("b"), "banjo", &b.file, &src).is_err());
        assert!(restore(&root.join("b"), "mm", "../x.zip", &src).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}

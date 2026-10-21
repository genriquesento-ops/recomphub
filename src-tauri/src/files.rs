//! Checking the files the player provides from their own copy, and telling them in plain
//! words what is missing. The checks are generic; catalog.json says which one each port
//! uses and with which values (see `catalog::Verifier`).

use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::catalog::{self, GameDef, Verifier};

#[derive(Serialize, Clone, Default)]
pub struct FilesStatus {
    pub ok: bool,
    /// One sentence for the library card.
    pub summary: String,
    /// What was found or what to fix, one item per line.
    pub details: Vec<String>,
    /// Where the files are (dump folder or stored ROM).
    pub path: Option<String>,
}

impl FilesStatus {
    fn ok(summary: impl Into<String>, details: Vec<String>, path: &Path) -> Self {
        Self { ok: true, summary: summary.into(), details, path: Some(path.display().to_string()) }
    }
    fn missing(summary: impl Into<String>, details: Vec<String>) -> Self {
        Self { ok: false, summary: summary.into(), details, path: None }
    }
}

pub fn status(game: &GameDef, install_dir: &Path) -> FilesStatus {
    match &game.files {
        Verifier::Ps4Dump { .. } => ps4::status(&game.files),
        Verifier::N64Rom { .. } => n64::status(&game.files, install_dir),
        Verifier::None => FilesStatus {
            ok: true,
            summary: "Nenhum arquivo necessário".into(),
            details: vec![],
            path: None,
        },
    }
}

/// Looks in `root` and the folders right below it for the ROM this port accepts. Only
/// N64 ROM files up to 64 MB are read, and only an exact match is returned. Nothing is copied.
pub fn find_rom(game: &GameDef, root: &Path) -> Option<PathBuf> {
    match &game.files {
        Verifier::N64Rom { .. } => n64::find(&game.files, root),
        _ => None,
    }
}

/// The player picked `path`: check it and, when it is right, hand it to the port.
pub fn provide(game: &GameDef, install_dir: &Path, path: &Path) -> FilesStatus {
    match &game.files {
        Verifier::Ps4Dump { .. } => ps4::provide(&game.files, path),
        Verifier::N64Rom { .. } => n64::provide(game, install_dir, path),
        Verifier::None => status(game, install_dir),
    }
}

/// The game's serial, for cover addresses with ${serial}.
pub fn serial(game: &GameDef) -> Option<String> {
    match &game.files {
        // The player's own dump says which edition it is (CUSA01623 or CUSA01715).
        Verifier::Ps4Dump { .. } => ps4::title_id(&game.files).or_else(|| game.serial.clone()),
        _ => game.serial.clone(),
    }
}

/// Where an N64: Recompiled port keeps its data (saves, mods, mods.json).
pub fn n64_data_dir(game: &GameDef, install_dir: &Path) -> Option<PathBuf> {
    n64::data_dir(&game.files, install_dir)
}

/// RT64's graphics API in an N64: Recompiled port's graphics.json: "Auto", "D3D12" or
/// "Vulkan". None for other ports, and before the port's first run (the file does not exist).
pub fn n64_graphics_api(game: &GameDef, install_dir: &Path) -> Option<String> {
    let path = n64_data_dir(game, install_dir)?.join("graphics.json");
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    Some(v.get("api_option").and_then(|a| a.as_str()).unwrap_or("Auto").to_string())
}

pub const GRAPHICS_APIS: &[&str] = &["Auto", "D3D12", "Vulkan"];

/// Changes only "api_option"; everything else the port wrote stays. Switching to Vulkan is
/// what fixed notebooks rendering on the integrated graphics (Zelda64Recomp issue 173).
pub fn set_n64_graphics_api(game: &GameDef, install_dir: &Path, api: &str) -> Result<(), String> {
    if !GRAPHICS_APIS.contains(&api) {
        return Err("Opção inválida.".into());
    }
    let path = n64_data_dir(game, install_dir).ok_or("Este port não usa graphics.json.")?.join("graphics.json");
    let bytes = std::fs::read(&path).map_err(|_| "Abra o jogo uma vez antes: ele cria as configurações de vídeo na primeira vez.".to_string())?;
    let mut v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| "As configurações de vídeo do jogo estão num formato que eu não conheço; não mexi nelas.".to_string())?;
    let obj = v.as_object_mut().ok_or("As configurações de vídeo do jogo estão num formato que eu não conheço; não mexi nelas.")?;
    obj.insert("api_option".into(), serde_json::Value::String(api.into()));
    let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.recomphub-tmp");
    std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Não consegui gravar as configurações de vídeo: {e}")
    })
}

/// The player's ROM as the port stored it (.z64 byte order), once provided.
pub fn n64_rom_file(game: &GameDef, install_dir: &Path) -> Option<PathBuf> {
    let Verifier::N64Rom { game_id, .. } = &game.files else { return None };
    Some(n64::data_dir(&game.files, install_dir)?.join(format!("{game_id}.z64")))
}

/// An N64: Recompiled port's cartridge save (`<data>/saves/<game_id>.bin`, N64 byte order).
pub fn n64_save_file(game: &GameDef, install_dir: &Path) -> Option<PathBuf> {
    let Verifier::N64Rom { game_id, .. } = &game.files else { return None };
    Some(n64::data_dir(&game.files, install_dir)?.join("saves").join(format!("{game_id}.bin")))
}

/// The folder where the port keeps the player's saves, when it exists.
pub fn save_dir(game: &GameDef, install_dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    match &game.files {
        // A custom "Saves folder" holds savedata\ (and the shader cache, left out).
        Verifier::Ps4Dump { .. } => candidates.extend(ps4::user_dir(&game.files).map(|d| d.join("savedata"))),
        Verifier::N64Rom { .. } => {
            if let Some(data) = n64::data_dir(&game.files, install_dir) {
                candidates.push(data.join("saves"));
                candidates.push(data);
            }
        }
        Verifier::None => {}
    }
    candidates.extend(game.saves.iter().filter_map(|t| catalog::expand(t, Some(install_dir))));
    candidates.into_iter().find(|d| d.is_dir())
}

/// A cover picture from the player's own files or the port's folder.
pub fn cover(game: &GameDef, install_dir: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Verifier::Ps4Dump { .. } = &game.files {
        if let Some(dump) = ps4::game_dir(&game.files) {
            candidates.push(dump.join("sce_sys").join("icon0.png"));
        }
    }
    if let Some(d) = install_dir {
        candidates.extend(game.cover_files.iter().map(|f| d.join(f)));
    }
    candidates.into_iter().find(|p| p.is_file())
}

mod sfo {
    //! PS4 param.sfo: a small key/value table.

    fn u16_at(b: &[u8], o: usize) -> Option<u16> {
        Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
    }
    fn u32_at(b: &[u8], o: usize) -> Option<u32> {
        Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
    }

    pub fn read_string(data: &[u8], key: &str) -> Option<String> {
        if data.get(0..4)? != b"\0PSF" {
            return None;
        }
        let keys = u32_at(data, 8)? as usize;
        let values = u32_at(data, 12)? as usize;
        let count = u32_at(data, 16)? as usize;
        for i in 0..count {
            let e = 20 + i * 16;
            let key_off = u16_at(data, e)? as usize;
            let len = u32_at(data, e + 4)? as usize;
            let val_off = u32_at(data, e + 12)? as usize;
            let k_start = keys + key_off;
            let k_end = k_start + data.get(k_start..)?.iter().position(|&c| c == 0)?;
            if &data[k_start..k_end] != key.as_bytes() {
                continue;
            }
            let raw = data.get(values + val_off..values + val_off + len)?;
            let text = String::from_utf8_lossy(raw);
            return Some(text.trim_end_matches('\0').to_string());
        }
        None
    }

    #[cfg(test)]
    mod tests {
        /// A minimal param.sfo with TITLE_ID and APP_VER.
        fn sample() -> Vec<u8> {
            let keys = b"APP_VER\0TITLE_ID\0".to_vec();
            let values = b"01.02\0\0\0CUSA01715\0\0\0".to_vec();
            let header = 20 + 2 * 16;
            let mut b = Vec::new();
            b.extend_from_slice(b"\0PSF");
            b.extend_from_slice(&0x101u32.to_le_bytes());
            b.extend_from_slice(&(header as u32).to_le_bytes());
            b.extend_from_slice(&((header + keys.len()) as u32).to_le_bytes());
            b.extend_from_slice(&2u32.to_le_bytes());
            for (key_off, len, val_off) in [(0u16, 6u32, 0u32), (8, 10, 8)] {
                b.extend_from_slice(&key_off.to_le_bytes());
                b.extend_from_slice(&0x0204u16.to_le_bytes());
                b.extend_from_slice(&len.to_le_bytes());
                b.extend_from_slice(&(len + 2).to_le_bytes());
                b.extend_from_slice(&val_off.to_le_bytes());
            }
            b.extend_from_slice(&keys);
            b.extend_from_slice(&values);
            b
        }

        #[test]
        fn reads_keys() {
            let s = sample();
            assert_eq!(super::read_string(&s, "TITLE_ID").as_deref(), Some("CUSA01715"));
            assert_eq!(super::read_string(&s, "APP_VER").as_deref(), Some("01.02"));
            assert_eq!(super::read_string(&s, "VERSION"), None);
            assert_eq!(super::read_string(b"not an sfo", "TITLE_ID"), None);
        }
    }
}

mod ps4 {
    use super::*;
    use serde_json::{Map, Value};

    struct Cfg<'a> {
        title_ids: &'a [String],
        tested: &'a [String],
        settings_file: &'a str,
        game_dir_key: &'a str,
        update_dir_key: Option<&'a str>,
        user_dir_key: Option<&'a str>,
    }

    fn cfg(v: &Verifier) -> Option<Cfg<'_>> {
        match v {
            Verifier::Ps4Dump { title_ids, tested_versions, settings_file, game_dir_key, update_dir_key, user_dir_key } => {
                Some(Cfg {
                    title_ids,
                    tested: tested_versions,
                    settings_file,
                    game_dir_key,
                    update_dir_key: update_dir_key.as_deref(),
                    user_dir_key: user_dir_key.as_deref(),
                })
            }
            _ => None,
        }
    }

    /// The port's own launcher settings: it reads the dump folder from here.
    fn settings(c: &Cfg) -> Map<String, Value> {
        catalog::expand(c.settings_file, None)
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    fn setting_dir(c: &Cfg, key: Option<&str>) -> Option<PathBuf> {
        settings(c).get(key?)?.as_str().filter(|s| !s.is_empty()).map(PathBuf::from)
    }

    pub fn game_dir(v: &Verifier) -> Option<PathBuf> {
        let c = cfg(v)?;
        setting_dir(&c, Some(c.game_dir_key))
    }

    /// A custom "Saves folder" chosen in the port's launcher.
    pub fn user_dir(v: &Verifier) -> Option<PathBuf> {
        let c = cfg(v)?;
        setting_dir(&c, c.user_dir_key)
    }

    pub fn title_id(v: &Verifier) -> Option<String> {
        let sfo = std::fs::read(game_dir(v)?.join("sce_sys").join("param.sfo")).ok()?;
        sfo::read_string(&sfo, "TITLE_ID").filter(|t| !t.is_empty())
    }

    struct Dump {
        title_id: String,
        version: String,
    }

    /// Checks a dump folder; Err is the list of problems in plain words.
    fn check_dump(c: &Cfg, dir: &Path) -> Result<Dump, Vec<String>> {
        let mut problems = Vec::new();
        if !dir.join("eboot.bin").is_file() {
            problems.push("Falta o arquivo eboot.bin.".to_string());
        }
        if !dir.join("sce_module").is_dir() {
            problems.push("Falta a pasta sce_module.".to_string());
        }
        let sfo = std::fs::read(dir.join("sce_sys").join("param.sfo")).ok();
        if sfo.is_none() {
            problems.push("Falta o arquivo sce_sys\\param.sfo (a identificação do jogo).".to_string());
        }
        if !problems.is_empty() {
            problems.push("Escolha a pasta do dump que contém eboot.bin, sce_module e sce_sys.".to_string());
            return Err(problems);
        }
        let sfo = sfo.unwrap();
        let title_id = sfo::read_string(&sfo, "TITLE_ID").unwrap_or_default();
        let version = sfo::read_string(&sfo, "APP_VER").unwrap_or_default();
        if !c.title_ids.contains(&title_id) {
            let name = sfo::read_string(&sfo, "TITLE").unwrap_or_else(|| "outro jogo".into());
            return Err(vec![
                format!("Esta pasta é de \"{name}\" ({title_id}), não do jogo deste port."),
                format!("O port aceita os códigos {}.", c.title_ids.join(", ")),
            ]);
        }
        Ok(Dump { title_id, version })
    }

    /// Picking the folder above the dump is common: look one level down too.
    fn find_dump(path: &Path) -> PathBuf {
        if path.join("eboot.bin").is_file() {
            return path.to_path_buf();
        }
        if let Ok(entries) = std::fs::read_dir(path) {
            for e in entries.flatten() {
                let p = e.path();
                if p.join("eboot.bin").is_file() && p.join("sce_sys").is_dir() {
                    return p;
                }
            }
        }
        path.to_path_buf()
    }

    fn describe(c: &Cfg, dir: &Path, dump: &Dump) -> FilesStatus {
        let mut details = vec![format!("Dump encontrado: {} · versão {}", dump.title_id, dump.version)];
        let mut version = dump.version.clone();
        if let Some(update) = setting_dir(c, c.update_dir_key) {
            if let Some(v) = std::fs::read(update.join("sce_sys").join("param.sfo"))
                .ok()
                .and_then(|b| sfo::read_string(&b, "APP_VER"))
            {
                details.push(format!("Atualização configurada no port: versão {v}"));
                version = v;
            }
        }
        if !c.tested.is_empty() && !c.tested.contains(&version) {
            details.push(format!(
                "A versão {version} não foi testada pelo port (testadas: {}). Pode funcionar, mas sem garantia.",
                c.tested.join(", ")
            ));
        }
        FilesStatus::ok(format!("{} · versão {version}", dump.title_id), details, dir)
    }

    pub fn status(v: &Verifier) -> FilesStatus {
        let Some(c) = cfg(v) else { return FilesStatus::default() };
        let Some(dir) = setting_dir(&c, Some(c.game_dir_key)) else {
            return FilesStatus::missing(
                "Falta indicar a pasta do jogo",
                vec!["O port precisa da pasta com o dump da sua cópia de PS4.".to_string()],
            );
        };
        match check_dump(&c, &dir) {
            Ok(dump) => describe(&c, &dir, &dump),
            Err(mut problems) => {
                problems.insert(0, format!("A pasta configurada ({}) não está completa:", dir.display()));
                FilesStatus::missing("Os arquivos do jogo estão incompletos", problems)
            }
        }
    }

    pub fn provide(v: &Verifier, path: &Path) -> FilesStatus {
        let Some(c) = cfg(v) else { return FilesStatus::default() };
        let dir = find_dump(path);
        let dump = match check_dump(&c, &dir) {
            Ok(d) => d,
            Err(problems) => return FilesStatus::missing("Esta pasta não serve", problems),
        };
        // Same setting the port's own launcher writes ("Game folder").
        let mut s = settings(&c);
        s.insert(c.game_dir_key.into(), Value::String(dir.display().to_string()));
        let saved = catalog::expand(c.settings_file, None).ok_or(()).and_then(|p| {
            std::fs::create_dir_all(p.parent().ok_or(())?).map_err(|_| ())?;
            let text = serde_json::to_string_pretty(&Value::Object(s)).map_err(|_| ())?;
            std::fs::write(&p, text).map_err(|_| ())
        });
        if saved.is_err() {
            return FilesStatus::missing(
                "Não consegui salvar a configuração do port",
                vec![format!("Não foi possível gravar {}.", c.settings_file)],
            );
        }
        describe(&c, &dir, &dump)
    }
}

mod n64 {
    use super::*;
    use xxhash_rust::xxh3::xxh3_64;

    const Z64_START: [u8; 4] = [0x80, 0x37, 0x12, 0x40];
    const MAX_ROM: u64 = 64 * 1024 * 1024;
    /// Stops a search in a folder full of ROMs from reading for minutes.
    const MAX_CHECKED: usize = 100;

    fn rom_name(path: &Path) -> bool {
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        ["z64", "n64", "v64"].contains(&ext.as_str())
    }

    fn matches(v: &Verifier, path: &Path) -> bool {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size == 0 || size > MAX_ROM {
            return false;
        }
        let Some(rom) = std::fs::read(path).ok().and_then(to_z64) else { return false };
        Some(xxh3_64(&rom)) == expected(v)
    }

    /// `root` and one level of folders below it; links to folders are not followed.
    pub fn find(v: &Verifier, root: &Path) -> Option<PathBuf> {
        let mut dirs = vec![(root.to_path_buf(), 0)];
        let mut checked = 0;
        while let Some((dir, depth)) = dirs.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let Ok(kind) = e.file_type() else { continue };
                let path = e.path();
                if kind.is_dir() && depth == 0 {
                    dirs.push((path, 1));
                } else if kind.is_file() && rom_name(&path) {
                    checked += 1;
                    if checked > MAX_CHECKED {
                        return None;
                    }
                    if matches(v, &path) {
                        return Some(path);
                    }
                }
            }
        }
        None
    }

    /// Where an N64: Recompiled port keeps its data: next to it with portable.txt, else
    /// %LOCALAPPDATA%\<program_id> (RecompFrontend's get_app_folder_path).
    pub fn data_dir(v: &Verifier, install_dir: &Path) -> Option<PathBuf> {
        let Verifier::N64Rom { program_id, .. } = v else { return None };
        if install_dir.join("portable.txt").is_file() {
            return Some(install_dir.to_path_buf());
        }
        std::env::var_os("LOCALAPPDATA").map(|a| PathBuf::from(a).join(program_id))
    }

    fn stored_path(v: &Verifier, install_dir: &Path) -> Option<PathBuf> {
        let Verifier::N64Rom { game_id, .. } = v else { return None };
        Some(data_dir(v, install_dir)?.join(format!("{game_id}.z64")))
    }

    fn expected(v: &Verifier) -> Option<u64> {
        let Verifier::N64Rom { xxh3, .. } = v else { return None };
        catalog::parse_hash(xxh3)
    }

    /// The port accepts byteswapped dumps (.v64, .n64) and stores them in .z64 order.
    pub fn to_z64(mut data: Vec<u8>) -> Option<Vec<u8>> {
        let s = data.get(0..4)?.to_vec();
        let swap = if s == Z64_START {
            0
        } else if s == [Z64_START[3], Z64_START[2], Z64_START[1], Z64_START[0]] {
            3
        } else if s == [Z64_START[1], Z64_START[0], Z64_START[3], Z64_START[2]] {
            1
        } else {
            return None;
        };
        if swap != 0 {
            for chunk in data.chunks_exact_mut(4) {
                let t = [chunk[0], chunk[1], chunk[2], chunk[3]];
                for (i, b) in t.iter().enumerate() {
                    chunk[i ^ swap] = *b;
                }
            }
        }
        Some(data)
    }

    pub fn status(v: &Verifier, install_dir: &Path) -> FilesStatus {
        let (Some(stored), Some(hash)) = (stored_path(v, install_dir), expected(v)) else {
            return FilesStatus::missing("Falta a ROM", vec![]);
        };
        let Verifier::N64Rom { edition, .. } = v else { return FilesStatus::default() };
        match std::fs::read(&stored) {
            Ok(data) if xxh3_64(&data) == hash => {
                FilesStatus::ok(format!("ROM {edition} conferida"), vec![format!("ROM {edition} conferida.")], &stored)
            }
            Ok(_) => FilesStatus::missing(
                "A ROM guardada está diferente",
                vec!["A ROM que o port guardou não confere. Escolha a ROM de novo.".to_string()],
            ),
            Err(_) => FilesStatus::missing(
                "Falta a ROM",
                vec![format!("O port precisa da ROM da sua cópia (versão {edition}).")],
            ),
        }
    }

    pub fn provide(game: &GameDef, install_dir: &Path, path: &Path) -> FilesStatus {
        let v = &game.files;
        let bad = |lines: Vec<String>| FilesStatus::missing("Esta ROM não serve", lines);
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ["zip", "7z", "rar"].contains(&ext.as_str()) {
            return bad(vec![
                "O arquivo está compactado.".to_string(),
                "Descompacte e escolha o arquivo .z64, .n64 ou .v64 de dentro dele.".to_string(),
            ]);
        }
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size == 0 || size > MAX_ROM {
            return bad(vec!["O arquivo não tem o tamanho de uma ROM de N64.".to_string()]);
        }
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => return bad(vec![format!("Não consegui ler o arquivo: {e}")]),
        };
        let Some(rom) = to_z64(data) else {
            return bad(vec!["Este arquivo não é uma ROM de Nintendo 64.".to_string()]);
        };
        if Some(xxh3_64(&rom)) != expected(v) {
            return bad(explain_wrong_rom(game, &rom));
        }
        let Some(stored) = stored_path(v, install_dir) else {
            return bad(vec!["Não encontrei a pasta de dados do Windows (LOCALAPPDATA).".to_string()]);
        };
        let dir = stored.parent().unwrap_or(install_dir);
        if let Err(e) = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&stored, &rom)) {
            return bad(vec![format!("Não consegui copiar a ROM para {}: {e}", dir.display())]);
        }
        status(v, install_dir)
    }

    /// The header says which game, region and revision the ROM is.
    pub fn explain_wrong_rom(game: &GameDef, rom: &[u8]) -> Vec<String> {
        let Verifier::N64Rom { edition, region, revision, name_hint, .. } = &game.files else { return vec![] };
        let name = rom
            .get(0x20..0x34)
            .map(|b| String::from_utf8_lossy(b).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string())
            .unwrap_or_default();
        let found_region = rom.get(0x3E).copied();
        let region_words = match found_region {
            Some(b'E') => "norte-americana (USA)",
            Some(b'P') => "europeia",
            Some(b'J') => "japonesa",
            Some(b'U') => "australiana",
            _ => "de região desconhecida",
        };
        let found_revision = rom.get(0x3F).copied().unwrap_or(0);
        let mut lines = Vec::new();
        if !name.to_lowercase().contains(name_hint.as_str()) {
            lines.push(format!("Esta ROM é de outro jogo (\"{name}\")."));
        } else if found_region != region.bytes().next() {
            lines.push(format!("Esta é a versão {region_words} do {}.", game.title));
        } else if found_revision != *revision {
            lines.push(format!("Esta é a revisão 1.{found_revision} do {}, não a esperada.", game.title));
        } else {
            lines.push(format!("É o {} certo, mas o arquivo está alterado (mod, patch ou dump com erro).", game.title));
        }
        lines.push(format!("O port só aceita a versão {edition}, sem modificações."));
        lines
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn rom(order: &[usize; 4]) -> Vec<u8> {
            let z64 = [0x80u8, 0x37, 0x12, 0x40, 1, 2, 3, 4];
            let mut out = Vec::new();
            for chunk in z64.chunks(4) {
                for &i in order {
                    out.push(chunk[i]);
                }
            }
            out
        }

        #[test]
        fn byteswapped_dumps_become_z64() {
            let z64 = rom(&[0, 1, 2, 3]);
            assert_eq!(to_z64(rom(&[0, 1, 2, 3])).unwrap(), z64);
            assert_eq!(to_z64(rom(&[1, 0, 3, 2])).unwrap(), z64, ".v64");
            assert_eq!(to_z64(rom(&[3, 2, 1, 0])).unwrap(), z64, ".n64");
            assert!(to_z64(b"PK\x03\x04zip!".to_vec()).is_none());
        }

        #[test]
        fn wrong_region_is_named() {
            let (_, games, _) = catalog::parse(catalog::BUILTIN).unwrap();
            let banjo = games.iter().find(|g| g.id == "banjo").unwrap();
            let mut fake = vec![0u8; 0x40];
            fake[0x20..0x2D].copy_from_slice(b"Banjo-Kazooie");
            fake[0x3E] = b'P';
            let lines = explain_wrong_rom(banjo, &fake);
            assert!(lines[0].contains("europeia"), "{lines:?}");
        }

        #[test]
        fn finds_only_the_exact_rom_in_a_folder() {
            let (_, games, _) = catalog::parse(catalog::BUILTIN).unwrap();
            let mut game = games.iter().find(|g| g.id == "banjo").unwrap().clone();
            let right = rom(&[0, 1, 2, 3]);
            let Verifier::N64Rom { xxh3, .. } = &mut game.files else { panic!() };
            *xxh3 = format!("0x{:016X}", xxh3_64(&right));

            let root = std::env::temp_dir().join(format!("recomphub-find-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("jogos").join("fundo")).unwrap();
            std::fs::write(root.join("outro.z64"), rom(&[0, 1, 2, 3]).iter().map(|b| b ^ 1).collect::<Vec<_>>()).unwrap();
            std::fs::write(root.join("jogos").join("fundo").join("longe.z64"), &right).unwrap();
            std::fs::write(root.join("certo.txt"), &right).unwrap();
            assert_eq!(super::super::find_rom(&game, &root), None, "too deep, wrong name or wrong ROM");

            let v64 = root.join("jogos").join("banjo.v64");
            std::fs::write(&v64, rom(&[1, 0, 3, 2])).unwrap();
            assert_eq!(super::super::find_rom(&game, &root), Some(v64));
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}

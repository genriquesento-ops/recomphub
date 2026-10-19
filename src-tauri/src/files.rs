//! Checking the files the player provides from their own copy, and telling them in plain
//! words what is missing.

use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::catalog::{GameDef, Kind};

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
    match game.kind {
        Kind::Gow3 => gow3::status(),
        Kind::Banjo => banjo::status(install_dir),
    }
}

/// The player picked `path`: check it and, when it is right, hand it to the port.
pub fn provide(game: &GameDef, install_dir: &Path, path: &Path) -> FilesStatus {
    match game.kind {
        Kind::Gow3 => gow3::provide(path),
        Kind::Banjo => banjo::provide(install_dir, path),
    }
}

/// The folder where the port keeps the player's saves, when it exists.
pub fn save_dir(game: &GameDef, install_dir: &Path) -> Option<PathBuf> {
    let dir = match game.kind {
        Kind::Gow3 => gow3::user_dir().unwrap_or_else(|| install_dir.join("user")),
        Kind::Banjo => {
            let data = banjo::data_dir(install_dir)?;
            let saves = data.join("saves");
            if saves.is_dir() { saves } else { data }
        }
    };
    dir.is_dir().then_some(dir)
}

/// A cover picture from the player's own files or the port's folder.
pub fn cover(game: &GameDef, install_dir: Option<&Path>) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    match game.kind {
        Kind::Gow3 => {
            if let Some(dump) = gow3::game_dir() {
                candidates.push(dump.join("sce_sys").join("icon0.png"));
            }
            if let Some(d) = install_dir {
                candidates.push(d.join("launcher").join("gow3.png"));
            }
        }
        Kind::Banjo => {
            if let Some(d) = install_dir {
                candidates.push(d.join("assets").join("Banjo.svg"));
            }
        }
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
}

mod gow3 {
    use super::*;
    use serde_json::{Map, Value};

    const TITLE_IDS: &[&str] = &["CUSA01623", "CUSA01715"];
    const TESTED_VERSIONS: &[&str] = &["01.00", "01.02"];

    /// The port's own launcher settings: it reads the dump folder from here.
    fn settings_path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("gow3-launcher").join("settings.json"))
    }

    fn settings() -> Map<String, Value> {
        settings_path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    fn setting_dir(key: &str) -> Option<PathBuf> {
        settings().get(key)?.as_str().filter(|s| !s.is_empty()).map(PathBuf::from)
    }

    pub fn game_dir() -> Option<PathBuf> {
        setting_dir("game_dir")
    }

    /// A custom "Saves folder" chosen in the port's launcher.
    pub fn user_dir() -> Option<PathBuf> {
        setting_dir("user_dir")
    }

    struct Dump {
        title_id: String,
        version: String,
    }

    /// Checks a dump folder; Err is the list of problems in plain words.
    fn check_dump(dir: &Path) -> Result<Dump, Vec<String>> {
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
        if !TITLE_IDS.contains(&title_id.as_str()) {
            let name = sfo::read_string(&sfo, "TITLE").unwrap_or_else(|| "outro jogo".into());
            return Err(vec![
                format!("Esta pasta é de \"{name}\" ({title_id}), não do God of War III Remastered."),
                "O port aceita os códigos CUSA01623 (EUA) e CUSA01715 (Europa).".to_string(),
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

    fn describe(dir: &Path, dump: &Dump) -> FilesStatus {
        let mut details = vec![format!("Dump encontrado: {} · versão {}", dump.title_id, dump.version)];
        let mut version = dump.version.clone();
        if let Some(update) = setting_dir("update_dir") {
            if let Some(v) = std::fs::read(update.join("sce_sys").join("param.sfo"))
                .ok()
                .and_then(|b| sfo::read_string(&b, "APP_VER"))
            {
                details.push(format!("Atualização configurada no port: versão {v}"));
                version = v;
            }
        }
        if !TESTED_VERSIONS.contains(&version.as_str()) {
            details.push(format!(
                "A versão {version} não foi testada pelo port (testadas: 01.00 e 01.02). Pode funcionar, mas sem garantia."
            ));
        }
        FilesStatus::ok(format!("{} · versão {version}", dump.title_id), details, dir)
    }

    pub fn status() -> FilesStatus {
        let Some(dir) = game_dir() else {
            return FilesStatus::missing(
                "Falta indicar a pasta do jogo",
                vec!["O port precisa da pasta com o dump da sua cópia de PS4.".to_string()],
            );
        };
        match check_dump(&dir) {
            Ok(dump) => describe(&dir, &dump),
            Err(mut problems) => {
                problems.insert(0, format!("A pasta configurada ({}) não está completa:", dir.display()));
                FilesStatus::missing("Os arquivos do jogo estão incompletos", problems)
            }
        }
    }

    pub fn provide(path: &Path) -> FilesStatus {
        let dir = find_dump(path);
        let dump = match check_dump(&dir) {
            Ok(d) => d,
            Err(problems) => return FilesStatus::missing("Esta pasta não serve", problems),
        };
        // Same setting the port's own launcher writes ("Game folder").
        let mut s = settings();
        s.insert("game_dir".into(), Value::String(dir.display().to_string()));
        let saved = settings_path().ok_or(()).and_then(|p| {
            std::fs::create_dir_all(p.parent().unwrap()).map_err(|_| ())?;
            let text = serde_json::to_string_pretty(&Value::Object(s)).map_err(|_| ())?;
            std::fs::write(&p, text).map_err(|_| ())
        });
        if saved.is_err() {
            return FilesStatus::missing(
                "Não consegui salvar a configuração do port",
                vec!["Não foi possível gravar %APPDATA%\\gow3-launcher\\settings.json.".to_string()],
            );
        }
        describe(&dir, &dump)
    }
}

mod banjo {
    use super::*;
    use xxhash_rust::xxh3::xxh3_64;

    /// XXH3 of the North American 1.0 ROM, as the port checks it (src/main/main.cpp).
    const ROM_HASH: u64 = 0x1B67585D56E07F8C;
    const STORED_NAME: &str = "bk.n64.us.1.0.z64";
    const Z64_START: [u8; 4] = [0x80, 0x37, 0x12, 0x40];

    /// Where the port keeps its data: next to it with portable.txt, else %LOCALAPPDATA%.
    pub fn data_dir(install_dir: &Path) -> Option<PathBuf> {
        if install_dir.join("portable.txt").is_file() {
            return Some(install_dir.to_path_buf());
        }
        std::env::var_os("LOCALAPPDATA").map(|a| PathBuf::from(a).join("BanjoRecompiled"))
    }

    /// The port accepts byteswapped dumps (.v64, .n64) and stores them in .z64 order.
    fn to_z64(mut data: Vec<u8>) -> Option<Vec<u8>> {
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

    pub fn status(install_dir: &Path) -> FilesStatus {
        let Some(dir) = data_dir(install_dir) else {
            return FilesStatus::missing("Falta a ROM", vec![]);
        };
        let stored = dir.join(STORED_NAME);
        match std::fs::read(&stored) {
            Ok(data) if xxh3_64(&data) == ROM_HASH => FilesStatus::ok(
                "ROM USA 1.0 conferida",
                vec!["ROM norte-americana 1.0 conferida.".to_string()],
                &stored,
            ),
            Ok(_) => FilesStatus::missing(
                "A ROM guardada está diferente",
                vec!["A ROM que o port guardou não confere. Escolha a ROM de novo.".to_string()],
            ),
            Err(_) => FilesStatus::missing(
                "Falta a ROM",
                vec!["O port precisa da ROM da sua cópia (Banjo-Kazooie USA 1.0).".to_string()],
            ),
        }
    }

    pub fn provide(install_dir: &Path, path: &Path) -> FilesStatus {
        let bad = |lines: Vec<String>| FilesStatus::missing("Esta ROM não serve", lines);
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ["zip", "7z", "rar"].contains(&ext.as_str()) {
            return bad(vec![
                "O arquivo está compactado.".to_string(),
                "Descompacte e escolha o arquivo .z64, .n64 ou .v64 de dentro dele.".to_string(),
            ]);
        }
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size == 0 || size > 64 * 1024 * 1024 {
            return bad(vec!["O arquivo não tem o tamanho de uma ROM de N64.".to_string()]);
        }
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => return bad(vec![format!("Não consegui ler o arquivo: {e}")]),
        };
        let Some(rom) = to_z64(data) else {
            return bad(vec!["Este arquivo não é uma ROM de Nintendo 64.".to_string()]);
        };
        if xxh3_64(&rom) != ROM_HASH {
            return bad(explain_wrong_rom(&rom));
        }
        let Some(dir) = data_dir(install_dir) else {
            return bad(vec!["Não encontrei a pasta de dados do Windows (LOCALAPPDATA).".to_string()]);
        };
        let stored = dir.join(STORED_NAME);
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&stored, &rom)) {
            return bad(vec![format!("Não consegui copiar a ROM para {}: {e}", dir.display())]);
        }
        status(install_dir)
    }

    /// The header says which game, region and revision the ROM is.
    fn explain_wrong_rom(rom: &[u8]) -> Vec<String> {
        let name = rom
            .get(0x20..0x34)
            .map(|b| String::from_utf8_lossy(b).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string())
            .unwrap_or_default();
        let region = match rom.get(0x3E) {
            Some(b'E') => "norte-americana (USA)",
            Some(b'P') => "europeia",
            Some(b'J') => "japonesa",
            Some(b'U') => "australiana",
            _ => "de região desconhecida",
        };
        let revision = rom.get(0x3F).copied().unwrap_or(0);
        let is_banjo = name.to_lowercase().contains("banjo");
        let mut lines = Vec::new();
        if !is_banjo {
            lines.push(format!("Esta ROM é de outro jogo (\"{name}\")."));
        } else if rom.get(0x3E) != Some(&b'E') {
            lines.push(format!("Esta é a versão {region} do Banjo-Kazooie."));
        } else if revision != 0 {
            lines.push(format!("Esta é a revisão 1.{revision} do Banjo-Kazooie, não a 1.0."));
        } else {
            lines.push("É o Banjo-Kazooie USA 1.0, mas o arquivo está alterado (mod, patch ou dump com erro).".to_string());
        }
        lines.push("O port só aceita a versão norte-americana (USA) 1.0, sem modificações.".to_string());
        lines
    }
}

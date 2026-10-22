//! The games RecompHUB knows, read from catalog.json (the copy built into RecompHUB, or a
//! newer revision downloaded from the project's repository). Each entry says where the
//! port is published, how to recognise an installed copy, and how to check the files the
//! player provides from their own copy. Adding a port is a data change, not a code change.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// The catalog built into this version of RecompHUB.
pub const BUILTIN: &str = include_str!("../catalog.json");
/// Format this version understands; catalogs of a newer format are ignored.
pub const FORMAT: u32 = 1;

/// A page that helps the player make their own copy of the game files.
#[derive(Deserialize, Serialize, Clone)]
pub struct GuideLink {
    pub label: String,
    pub url: String,
}

/// How the files from the player's own copy are checked.
#[derive(Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Verifier {
    /// An N64: Recompiled port: the player picks a ROM, RecompHUB checks it the way the
    /// port does (XXH3 of the .z64 byte order) and copies it where the port looks for it.
    N64Rom {
        /// XXH3-64 from the port's own source (GameEntry::rom_hash), as "0x…".
        xxh3: String,
        /// Folder name under %LOCALAPPDATA% (programconfig program_id).
        program_id: String,
        /// The stored ROM is "<game_id>.z64" (GameEntry::stored_filename).
        game_id: String,
        /// Words for messages, such as "norte-americana (USA) 1.0".
        edition: String,
        /// Header country code of the right ROM ('E' is USA) and its revision byte.
        region: String,
        revision: u8,
        /// Lowercase word expected in the ROM's internal name, to tell another game apart.
        name_hint: String,
    },
    /// A PS4 game: the player picks the dump folder; RecompHUB checks param.sfo and writes
    /// the folder into the port's own settings file.
    Ps4Dump {
        title_ids: Vec<String>,
        tested_versions: Vec<String>,
        /// Where the port's launcher keeps its settings. No {install} here: RecompHUB versions
        /// before 0.23 refuse it, and would drop the whole game from their catalog.
        settings_file: String,
        /// The settings next to the port's program ({install} allowed), which newer versions of
        /// the port use (SpartanPort). Read and written first; `settings_file` stays as the
        /// fallback until the port's launcher copies it over on its first start.
        #[serde(default)]
        portable_settings_file: Option<String>,
        game_dir_key: String,
        #[serde(default)]
        update_dir_key: Option<String>,
        #[serde(default)]
        user_dir_key: Option<String>,
    },
    /// Nothing to provide (a free game).
    None,
}

#[derive(Deserialize, Clone)]
pub struct ModsDef {
    pub thunderstore: String,
    pub game_id: String,
}

#[derive(Deserialize, Clone)]
pub struct GameDef {
    pub id: String,
    /// Central de Mods (mods.rs): the game's Thunderstore community and the game_id its
    /// .nrm files declare. None: no mods.
    #[serde(default)]
    pub mods: Option<ModsDef>,
    /// Instruction sets the port's code needs from the processor (pcinfo::CPU_FEATURES); a PC
    /// without them is told before installing.
    #[serde(default)]
    pub cpu_features: Vec<String>,
    pub title: String,
    pub family: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub port_name: String,
    pub port_authors: String,
    pub project_url: String,
    /// GitHub "owner/name" of the official project.
    pub repo: String,
    /// The Windows asset of a release: its name starts and ends with these.
    pub asset_prefix: String,
    pub asset_suffix: String,
    /// Started by the Play button (relative to the install folder).
    pub exe: String,
    /// Icon for shortcuts (relative to the install folder; an .exe uses its own icon).
    pub icon: String,
    /// The program's name before the project renamed it (installations not yet updated).
    #[serde(default)]
    pub legacy_exe: Option<String>,
    /// Markers of an installation from before the rename.
    #[serde(default)]
    pub legacy_markers: Vec<String>,
    /// Folders where the port keeps its compiled shaders ({install} allowed): kept on
    /// uninstall like the saves, and "Limpar cache de shaders" on the game page.
    #[serde(default)]
    pub shader_cache: Vec<String>,
    /// The port's own log files ({install} allowed); the end of each goes into the problem
    /// report when the game closes with an error (diagnose.rs).
    #[serde(default)]
    pub logs: Vec<String>,
    /// Files whose presence identifies an installed copy of this port.
    pub markers: Vec<String>,
    /// Files kept from the old copy when updating (paths or folder prefixes, with '/').
    #[serde(default)]
    pub keep_on_update: Vec<String>,
    pub requirements: String,
    pub needs: String,
    pub files_help: String,
    pub files: Verifier,
    /// Where saves live, as path templates (see `expand`); the first that exists is opened.
    #[serde(default)]
    pub saves: Vec<String>,
    /// Pictures in the port's folder that can serve as cover.
    #[serde(default)]
    pub cover_files: Vec<String>,
    /// Curated cover addresses, tried before the player's own templates.
    #[serde(default)]
    pub cover_urls: Vec<String>,
    /// Serial for ${serial} cover templates, when the files do not tell it.
    #[serde(default)]
    pub serial: Option<String>,
    #[serde(default)]
    pub guide: Vec<String>,
    #[serde(default)]
    pub guide_links: Vec<GuideLink>,
}

#[derive(Deserialize)]
pub struct Catalog {
    pub format: u32,
    pub revision: u32,
    pub ports: Vec<GameDef>,
}

impl GameDef {
    /// The folder holds this port: every marker, or every legacy marker (an installation from
    /// before the project's rename, which an update brings to the new names).
    pub fn matches(&self, dir: &Path) -> bool {
        let all = |list: &[String]| !list.is_empty() && list.iter().all(|m| dir.join(m).is_file());
        all(&self.markers) || all(&self.legacy_markers)
    }

    /// The program the Play button opens in `dir`: the current name, or the old one in an
    /// installation from before the rename.
    pub fn program_in(&self, dir: &Path) -> String {
        match &self.legacy_exe {
            Some(old) if !dir.join(&self.exe).is_file() && dir.join(old).is_file() => old.clone(),
            _ => self.exe.clone(),
        }
    }

    /// "folder" or "file": what the player picks when providing the game files.
    pub fn files_kind(&self) -> &'static str {
        match self.files {
            Verifier::Ps4Dump { .. } => "folder",
            Verifier::N64Rom { .. } => "file",
            Verifier::None => "",
        }
    }

    /// Every address the game page may open.
    pub fn links(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.project_url.as_str()).chain(self.guide_links.iter().map(|l| l.url.as_str()))
    }
}

/// A relative path that stays inside its folder.
fn safe_relative(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && !path.is_absolute()
        && !p.contains(':')
        && path.components().all(|c| matches!(c, std::path::Component::Normal(_)))
}

fn slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Everything RecompHUB relies on before trusting an entry, most of all for a catalog
/// that came over the network.
pub fn validate(g: &GameDef) -> Result<(), String> {
    let err = |what: &str| Err(format!("{}: {what}", g.id));
    if !slug(&g.id) || g.id.starts_with("pc-") {
        return err("id inválido");
    }
    let repo_ok = g.repo.split('/').count() == 2
        && g.repo.chars().all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
        && !g.repo.contains("..");
    if !repo_ok {
        return err("repo inválido");
    }
    if !g.links().chain(g.cover_urls.iter().map(String::as_str)).all(|u| u.starts_with("https://")) {
        return err("links precisam ser https");
    }
    if let Some(m) = &g.mods {
        if !crate::mods::valid_community(&m.thunderstore) || !slug(&m.game_id) || !matches!(g.files, Verifier::N64Rom { .. }) {
            return err("mods inválidos");
        }
    }
    if !g.cpu_features.iter().all(|f| crate::pcinfo::CPU_FEATURES.contains(&f.as_str())) {
        return err("cpu_features desconhecido");
    }
    if g.markers.is_empty() || !g.markers.iter().chain(&g.legacy_markers).all(|m| safe_relative(m)) {
        return err("markers inválidos");
    }
    let rel_ok = |list: &[String]| list.iter().all(|p| safe_relative(p.trim_end_matches('/')));
    if !safe_relative(&g.exe)
        || !g.legacy_exe.as_deref().map(safe_relative).unwrap_or(true)
        || !safe_relative(&g.icon)
        || !rel_ok(&g.keep_on_update)
        || !rel_ok(&g.cover_files)
    {
        return err("caminho fora da pasta do port");
    }
    let is_exe = |e: &str| e.to_lowercase().ends_with(".exe");
    if !is_exe(&g.exe) || !g.legacy_exe.as_deref().map(is_exe).unwrap_or(true) || g.asset_prefix.is_empty() || !g.asset_suffix.ends_with(".zip") {
        return err("programa ou arquivo de release inválido");
    }
    if !g.shader_cache.iter().all(|s| expand(s, Some(Path::new("C:\\x"))).is_some()) {
        return err("pasta de shaders inválida");
    }
    if !g.logs.iter().all(|s| expand(s, Some(Path::new("C:\\x"))).is_some()) {
        return err("log inválido");
    }
    if !g.saves.iter().all(|s| expand(s, Some(Path::new("C:\\x"))).is_some()) {
        return err("pasta de saves inválida");
    }
    match &g.files {
        Verifier::N64Rom { xxh3, program_id, game_id, region, .. } => {
            if parse_hash(xxh3).is_none() {
                return err("xxh3 inválido");
            }
            let name_ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
            if !name_ok(program_id) || !name_ok(game_id) || region.len() != 1 {
                return err("dados da ROM inválidos");
            }
        }
        Verifier::Ps4Dump { title_ids, settings_file, portable_settings_file, .. } => {
            if portable_settings_file.as_deref().map(|p| expand(p, Some(Path::new("C:\\x"))).is_none()).unwrap_or(false) {
                return err("portable_settings_file inválido");
            }
            if title_ids.is_empty() || expand(settings_file, None).is_none() {
                return err("dados do dump inválidos");
            }
        }
        Verifier::None => {}
    }
    Ok(())
}

pub fn parse_hash(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim().trim_start_matches("0x").trim_start_matches("0X"), 16).ok()
}

/// Parses and validates a whole catalog. Entries that fail are left out, never trusted.
pub fn parse(text: &str) -> Result<(u32, Vec<GameDef>, Vec<String>), String> {
    let c: Catalog = serde_json::from_str(text).map_err(|e| format!("catálogo ilegível: {e}"))?;
    if c.format != FORMAT {
        return Err(format!("formato {} não suportado", c.format));
    }
    let mut ok: Vec<GameDef> = Vec::new();
    let mut rejected = Vec::new();
    for g in c.ports {
        match validate(&g) {
            Ok(()) if !ok.iter().any(|o| o.id == g.id) => ok.push(g),
            Ok(()) => rejected.push(format!("{}: id repetido", g.id)),
            Err(e) => rejected.push(e),
        }
    }
    Ok((c.revision, ok, rejected))
}

/// Path templates in the catalog: %APPDATA%, %LOCALAPPDATA%, %USERPROFILE% and {install}.
/// Anything else is refused, so a catalog cannot point at arbitrary places.
pub fn expand(template: &str, install: Option<&Path>) -> Option<PathBuf> {
    let mut s = template.to_string();
    for var in ["APPDATA", "LOCALAPPDATA", "USERPROFILE"] {
        let key = format!("%{var}%");
        if s.contains(&key) {
            let value = std::env::var(var).ok()?;
            s = s.replace(&key, &value);
        }
    }
    if s.contains("{install}") {
        s = s.replace("{install}", &install?.display().to_string());
    }
    if s.contains('%') || s.contains('{') || s.contains("..") || !Path::new(&s).is_absolute() {
        return None;
    }
    Some(PathBuf::from(s))
}

struct Current {
    revision: u32,
    games: &'static [GameDef],
}

static CURRENT: RwLock<Option<Current>> = RwLock::new(None);

/// Installs a catalog. Each one is kept for the life of the program (a few kilobytes,
/// replaced at most once per start) so references to its entries stay valid.
pub fn install(revision: u32, games: Vec<GameDef>) {
    let games: &'static [GameDef] = Box::leak(games.into_boxed_slice());
    *CURRENT.write().unwrap() = Some(Current { revision, games });
}

pub fn revision() -> u32 {
    CURRENT.read().unwrap().as_ref().map(|c| c.revision).unwrap_or(0)
}

pub fn games() -> &'static [GameDef] {
    CURRENT.read().unwrap().as_ref().map(|c| c.games).unwrap_or(&[])
}

pub fn get(id: &str) -> Option<&'static GameDef> {
    games().iter().find(|g| g.id == id)
}

/// Which port is in `dir`: the folder itself, or one folder below it (the player often
/// picks the folder that contains the port's folder).
pub fn identify(dir: &Path) -> Option<(&'static GameDef, PathBuf)> {
    for g in games() {
        if g.matches(dir) {
            return Some((g, dir.to_path_buf()));
        }
    }
    let entries = std::fs::read_dir(dir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        for g in games() {
            if g.matches(&p) {
                return Some((g, p));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_is_valid() {
        let (revision, games, rejected) = parse(BUILTIN).expect("catalog.json parses");
        assert!(rejected.is_empty(), "entradas recusadas: {rejected:?}");
        assert!(revision >= 1);
        assert!(games.len() >= 2);
    }

    #[test]
    fn templates_stay_in_known_places() {
        assert!(expand("{install}\\user", Some(Path::new("C:\\Games\\gow3"))).is_some());
        assert!(expand("%TEMP%\\x", None).is_none());
        assert!(expand("{install}\\..\\..\\Windows", Some(Path::new("C:\\g"))).is_none());
        assert!(expand("relative\\path", None).is_none());
    }

    #[test]
    fn renamed_port_keeps_old_installations() {
        let (_, games, _) = parse(BUILTIN).unwrap();
        let g = games.iter().find(|g| g.id == "gow3").unwrap();
        let dir = std::env::temp_dir().join(format!("recomphub-rename-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        for f in g.markers.iter().chain(g.legacy_exe.iter()) {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        // Before the update: recognised, and Play opens the old program.
        assert!(g.matches(&dir));
        assert_eq!(g.program_in(&dir), g.legacy_exe.clone().unwrap());
        // After the update: the new program wins.
        std::fs::write(dir.join(&g.exe), b"x").unwrap();
        assert_eq!(g.program_in(&dir), g.exe);
        // A folder with only the legacy markers of another kind of port still matches by them.
        let other = GameDef { markers: vec!["novo.exe".into()], legacy_markers: vec!["run.py".into()], ..g.clone() };
        assert!(other.matches(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relative_paths_cannot_escape() {
        assert!(safe_relative("bin/gow3-probe.exe"));
        assert!(!safe_relative("../evil.exe"));
        assert!(!safe_relative("C:\\Windows\\x.exe"));
        assert!(!safe_relative("\\x.exe"));
    }
}

//! The player's library: which games are installed where, and the play time. Saved as
//! library.json in RecompHUB's data folder.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// A game from the catalog, recognised by its port's files.
    #[default]
    Port,
    /// Any other PC game the player added by pointing at its program. RecompHUB does not
    /// try to tell where it came from.
    PcGame,
    /// A community port installed from its GitHub release without RecompHUB's checks
    /// (aba Comunidade, instalação experimental).
    Community,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Entry {
    #[serde(default)]
    pub kind: EntryKind,
    /// Port: the port's folder. PC game: the folder of its program.
    pub install_dir: String,
    /// PC game only: the program's file name inside `install_dir`.
    #[serde(default)]
    pub exe: Option<String>,
    /// PC game only: the name the player sees.
    #[serde(default)]
    pub title: Option<String>,
    /// Community port: GitHub "owner/name" it was installed from.
    #[serde(default)]
    pub repo: Option<String>,
    /// The player's own labels, for filtering the library.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Release tag installed by RecompHUB; None for a copy added from a folder.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub total_seconds: u64,
    /// Unix time the last session ended.
    #[serde(default)]
    pub last_played: Option<u64>,
    /// No "versão nova" notice for this port (as Heroic's ignoreGameUpdates). Turned on when
    /// the player picks an older version, so it is not swapped back behind their back.
    #[serde(default)]
    pub ignore_updates: bool,
    /// Shown first in the library.
    #[serde(default)]
    pub favorite: bool,
    /// Left out of the library unless "Mostrar ocultos" is on.
    #[serde(default)]
    pub hidden: bool,
    /// Extra command-line arguments for the game's program (as Heroic's launcherArgs).
    #[serde(default)]
    pub launch_args: String,
    /// Extra environment variables for the game (as Heroic's enviromentOptions).
    #[serde(default)]
    pub env: Vec<EnvVar>,
    /// Where the player is with the game (as Playnite's completion status).
    #[serde(default)]
    pub completion: Completion,
    /// Sessions of at least history::MIN_SESSION_SECS.
    #[serde(default)]
    pub play_count: u32,
    /// The player's own notes about the game.
    #[serde(default)]
    pub notes: String,
}

/// As Playnite's completion statuses. `None`: not chosen; the page shows "Não jogado" or
/// "Jogando" from the play time, and the first session sets `Playing`.
#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Completion {
    #[default]
    None,
    NotPlayed,
    Planned,
    Playing,
    OnHold,
    Beaten,
    Completed,
    Abandoned,
}

impl Completion {
    pub fn parse(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Library {
    #[serde(default)]
    pub games: BTreeMap<String, Entry>,
    /// Play time kept for games removed from the library, so adding them back keeps it.
    #[serde(default)]
    pub removed: BTreeMap<String, Entry>,
}

impl Library {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Writes a temporary file first so a crash never leaves half a library.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    pub fn add(&mut self, id: &str, install_dir: &Path, version: Option<String>) {
        let mut entry = self.games.remove(id).or_else(|| self.removed.remove(id)).unwrap_or_default();
        entry.install_dir = install_dir.display().to_string();
        entry.version = version;
        self.games.insert(id.to_string(), entry);
    }

    pub fn add_pc_game(&mut self, id: &str, exe: &Path, title: &str) {
        let mut entry = self.removed.remove(id).unwrap_or_default();
        entry.kind = EntryKind::PcGame;
        entry.install_dir = exe.parent().map(|p| p.display().to_string()).unwrap_or_default();
        entry.exe = exe.file_name().map(|n| n.to_string_lossy().to_string());
        entry.title = Some(title.to_string());
        entry.version = None;
        self.games.insert(id.to_string(), entry);
    }

    pub fn add_community(&mut self, id: &str, dir: &Path, repo: &str, title: &str, tag: &str, exe: Option<String>) {
        let mut entry = self.games.remove(id).or_else(|| self.removed.remove(id)).unwrap_or_default();
        entry.kind = EntryKind::Community;
        entry.install_dir = dir.display().to_string();
        entry.repo = Some(repo.to_string());
        entry.title = Some(title.to_string());
        entry.version = Some(tag.to_string());
        // An update keeps the program the player chose, when it still exists.
        if entry.exe.as_ref().map(|e| !dir.join(e).is_file()).unwrap_or(true) {
            entry.exe = exe;
        }
        self.games.insert(id.to_string(), entry);
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(e) = self.games.remove(id) {
            self.removed.insert(id.to_string(), e);
        }
    }

    pub fn install_dir(&self, id: &str) -> Option<PathBuf> {
        self.games.get(id).map(|e| PathBuf::from(&e.install_dir))
    }
}

/// "2026-10-19" for a Unix time (UTC), for backup file names.
pub fn date(unix: u64) -> String {
    // Civil date from days (Howard Hinnant's algorithm).
    let z = (unix / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates() {
        assert_eq!(super::date(0), "1970-01-01");
        assert_eq!(super::date(1_760_832_000), "2025-10-19");
        assert_eq!(super::date(1_709_164_800), "2024-02-29");
    }
}

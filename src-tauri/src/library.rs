//! The player's library: which games are installed where, and the play time. Saved as
//! library.json in RecompHUB's data folder.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Entry {
    pub install_dir: String,
    /// Release tag installed by RecompHUB; None for a copy added from a folder.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub total_seconds: u64,
    /// Unix time the last session ended.
    #[serde(default)]
    pub last_played: Option<u64>,
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

    pub fn remove(&mut self, id: &str) {
        if let Some(e) = self.games.remove(id) {
            self.removed.insert(id.to_string(), e);
        }
    }

    pub fn install_dir(&self, id: &str) -> Option<PathBuf> {
        self.games.get(id).map(|e| PathBuf::from(&e.install_dir))
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

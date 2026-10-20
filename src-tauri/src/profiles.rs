//! Several players on one PC. The library (what is installed where) is shared; each profile
//! has its own play time, last session, favorites, hidden games, completion, play count,
//! notes, history and (later) achievements.
//!
//! The active profile's progress always lives in library.json, so the rest of RecompHUB
//! needs no change: switching profiles stores the current progress in
//! `profiles/<id>.json` and loads the other one's. Switching is refused while a game runs.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::library::{Completion, Entry, Library};

pub const MAIN: &str = "principal";
pub const COLORS: &[&str] = &["#e0a32e", "#4cc38a", "#6aa7ff", "#e5675a", "#b07cf0", "#f08cc0", "#4cc3c3", "#9aa1ae"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub color: String,
    pub created: u64,
    /// RetroAchievements account (ra.rs), read only. The key never goes back to the page.
    #[serde(default)]
    pub ra_user: String,
    #[serde(default)]
    pub ra_key: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Profiles {
    pub active: String,
    pub list: Vec<Profile>,
    /// Show "Quem vai jogar?" when RecompHUB opens (with more than one profile).
    #[serde(default = "yes")]
    pub ask_on_start: bool,
    /// Games whose saves are kept apart for each profile (swapped when the profile changes).
    #[serde(default)]
    pub separate_saves: Vec<String>,
}

fn yes() -> bool {
    true
}

impl Default for Profiles {
    fn default() -> Self {
        Self {
            active: MAIN.into(),
            list: vec![Profile { id: MAIN.into(), name: "Principal".into(), color: COLORS[0].into(), created: 0, ra_user: String::new(), ra_key: String::new() }],
            ask_on_start: true,
            separate_saves: Vec::new(),
        }
    }
}

/// What belongs to one player for one game.
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct Progress {
    #[serde(default)]
    pub total_seconds: u64,
    #[serde(default)]
    pub last_played: Option<u64>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub completion: Completion,
    #[serde(default)]
    pub play_count: u32,
    #[serde(default)]
    pub notes: String,
}

impl Progress {
    fn of(e: &Entry) -> Self {
        Self {
            total_seconds: e.total_seconds,
            last_played: e.last_played,
            favorite: e.favorite,
            hidden: e.hidden,
            completion: e.completion,
            play_count: e.play_count,
            notes: e.notes.clone(),
        }
    }

    fn put(self, e: &mut Entry) {
        e.total_seconds = self.total_seconds;
        e.last_played = self.last_played;
        e.favorite = self.favorite;
        e.hidden = self.hidden;
        e.completion = self.completion;
        e.play_count = self.play_count;
        e.notes = self.notes;
    }
}

impl Profiles {
    pub fn load(path: &Path) -> Self {
        let mut p: Self = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        if p.list.is_empty() {
            p = Self::default();
        }
        if !p.list.iter().any(|x| x.id == p.active) {
            p.active = p.list[0].id.clone();
        }
        p
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.list.iter().find(|p| p.id == id)
    }

    pub fn active(&self) -> &Profile {
        self.get(&self.active).unwrap_or(&self.list[0])
    }
}

/// A short id from the name, unique among the profiles.
pub fn new_id(profiles: &Profiles, name: &str, now: u64) -> String {
    let mut base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    base.truncate(24);
    if base.is_empty() || base == MAIN {
        base = format!("perfil-{}", now % 100_000);
    }
    let mut id = base.clone();
    let mut n = 2;
    while profiles.get(&id).is_some() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

pub fn clean_name(name: &str) -> Result<String, String> {
    let name: String = name.trim().chars().filter(|c| !c.is_control()).collect();
    if name.is_empty() {
        return Err("Escreva um nome para o perfil.".into());
    }
    if name.chars().count() > 30 {
        return Err("Use um nome de até 30 letras.".into());
    }
    Ok(name)
}

fn progress_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// Everyone's progress in the library now (games and removed games).
pub fn extract(lib: &Library) -> BTreeMap<String, Progress> {
    lib.games.iter().chain(lib.removed.iter()).map(|(id, e)| (id.clone(), Progress::of(e))).collect()
}

/// Puts a profile's progress into the library; games it never played start from zero.
pub fn apply(lib: &mut Library, mut progress: BTreeMap<String, Progress>) {
    for (id, e) in lib.games.iter_mut().chain(lib.removed.iter_mut()) {
        progress.remove(id).unwrap_or_default().put(e);
    }
}

pub fn load_progress(dir: &Path, id: &str) -> BTreeMap<String, Progress> {
    std::fs::read(progress_path(dir, id)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn store_progress(dir: &Path, id: &str, p: &BTreeMap<String, Progress>) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
    let tmp = progress_path(dir, id).with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, progress_path(dir, id)).map_err(|e| e.to_string())
}

/// Stores the active profile's progress and loads `to`'s into the library.
pub fn switch(lib: &mut Library, profiles: &mut Profiles, dir: &Path, to: &str) -> Result<(), String> {
    if profiles.get(to).is_none() {
        return Err("Perfil não encontrado.".into());
    }
    if profiles.active == to {
        return Ok(());
    }
    store_progress(dir, &profiles.active, &extract(lib))?;
    apply(lib, load_progress(dir, to));
    profiles.active = to.to_string();
    Ok(())
}

/// Where a profile's own saves wait while another profile plays: `<dir>/saves/<profile>/<game>/<label>`.
pub fn stash(dir: &Path, profile: &str, game: &str, label: &str) -> PathBuf {
    dir.join("saves").join(profile).join(game).join(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib_with(id: &str, secs: u64, fav: bool) -> Library {
        let mut lib = Library::default();
        lib.games.insert(id.into(), Entry { total_seconds: secs, favorite: fav, install_dir: "C:/x".into(), ..Default::default() });
        lib
    }

    #[test]
    fn each_profile_keeps_its_own_progress() {
        let dir = std::env::temp_dir().join(format!("recomphub-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut profiles = Profiles::default();
        let ana = Profile { id: "ana".into(), name: "Ana".into(), color: COLORS[1].into(), created: 1, ra_user: String::new(), ra_key: String::new() };
        profiles.list.push(ana);
        let mut lib = lib_with("banjo", 3600, true);

        switch(&mut lib, &mut profiles, &dir, "ana").unwrap();
        let e = &lib.games["banjo"];
        assert_eq!((e.total_seconds, e.favorite), (0, false), "a Ana começa do zero");
        assert_eq!(e.install_dir, "C:/x", "a instalação é a mesma");
        lib.games.get_mut("banjo").unwrap().total_seconds = 60;

        switch(&mut lib, &mut profiles, &dir, MAIN).unwrap();
        assert_eq!(lib.games["banjo"].total_seconds, 3600);
        assert!(lib.games["banjo"].favorite);
        switch(&mut lib, &mut profiles, &dir, "ana").unwrap();
        assert_eq!(lib.games["banjo"].total_seconds, 60);
        assert!(switch(&mut lib, &mut profiles, &dir, "ninguem").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ids_and_names() {
        let p = Profiles::default();
        assert_eq!(new_id(&p, "João Pedro!", 5), "jo-o-pedro");
        assert_eq!(new_id(&p, "Principal", 5), "perfil-5");
        assert!(clean_name("   ").is_err());
        assert!(clean_name(&"x".repeat(31)).is_err());
        assert_eq!(clean_name(" Ana ").unwrap(), "Ana");
    }
}

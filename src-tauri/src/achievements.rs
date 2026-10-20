//! RecompHUB's own achievements. The ports do not report achievements, and
//! RetroAchievements only accepts unlocks from its approved emulators, so RecompHUB never
//! unlocks anything there. Instead each achievement here has a trigger on a number RecompHUB
//! can read itself:
//! - play time, sessions and days played (every game, from the library and the history);
//! - progress read from the game's own save (read only), with the save layout taken from
//!   the game's public decompilation, as oni-save-parser does for Oxygen Not Included.
//!
//! The format follows RetroAchievements' (title, description, points, type, badge,
//! hidden); the definitions live in `achievements.json`, checked like `catalog.json`.
//! Unlocks are kept per profile in `achievements/<profile>.json`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const BUILTIN: &str = include_str!("../achievements.json");
const FORMAT: u32 = 1;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Trigger {
    pub stat: String,
    pub at_least: u64,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Def {
    pub id: String,
    pub title: String,
    pub description: String,
    pub points: u32,
    /// As RetroAchievements: "progression", "win_condition" or "missable".
    #[serde(rename = "type", default = "progression")]
    pub kind: String,
    #[serde(default)]
    pub badge: String,
    #[serde(default)]
    pub hidden: bool,
    pub trigger: Trigger,
}

fn progression() -> String {
    "progression".into()
}

#[derive(Deserialize, Clone, Debug)]
pub struct GameSet {
    pub reader: String,
    #[serde(default)]
    pub source: String,
    pub achievements: Vec<Def>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Defs {
    pub format: u32,
    #[serde(default)]
    pub revision: u32,
    pub generic: Vec<Def>,
    #[serde(default)]
    pub games: BTreeMap<String, GameSet>,
}

/// Stats each kind of trigger may use.
pub const GENERIC_STATS: &[&str] = &["play_count", "play_seconds", "longest_session", "days_played"];
pub const READERS: &[(&str, &[&str])] = &[("bk_eeprom", &["jiggies", "notes", "honeycombs", "mumbo_tokens"])];

pub fn parse(text: &str) -> Result<Defs, String> {
    let d: Defs = serde_json::from_str(text).map_err(|e| format!("conquistas ilegíveis: {e}"))?;
    if d.format != FORMAT {
        return Err(format!("formato de conquistas {} não suportado", d.format));
    }
    let check = |list: &[Def], stats: &[&str], ctx: &str| -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for a in list {
            if a.id.is_empty() || !a.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err(format!("{ctx}: id inválido \"{}\"", a.id));
            }
            if !seen.insert(&a.id) {
                return Err(format!("{ctx}: id repetido \"{}\"", a.id));
            }
            if !stats.contains(&a.trigger.stat.as_str()) {
                return Err(format!("{ctx}/{}: contador \"{}\" desconhecido", a.id, a.trigger.stat));
            }
            if a.title.is_empty() || a.points == 0 || a.points > 100 {
                return Err(format!("{ctx}/{}: título ou pontos inválidos", a.id));
            }
        }
        Ok(())
    };
    check(&d.generic, GENERIC_STATS, "generic")?;
    for (game, set) in &d.games {
        let stats = READERS.iter().find(|(r, _)| *r == set.reader).map(|(_, s)| *s).ok_or(format!("{game}: leitor \"{}\" desconhecido", set.reader))?;
        check(&set.achievements, stats, game)?;
        if set.achievements.iter().any(|a| d.generic.iter().any(|g| g.id == a.id)) {
            return Err(format!("{game}: id igual ao de uma conquista geral"));
        }
    }
    Ok(d)
}

pub fn builtin() -> &'static Defs {
    static DEFS: std::sync::OnceLock<Defs> = std::sync::OnceLock::new();
    DEFS.get_or_init(|| parse(BUILTIN).expect("achievements.json embutido inválido"))
}

/// Every achievement of a game: the general ones, then the game's own.
pub fn for_game(defs: &Defs, game: &str) -> (Vec<Def>, Option<&GameSet>) {
    let set = defs.games.get(game);
    let mut list = defs.generic.clone();
    if let Some(s) = set {
        list.extend(s.achievements.iter().cloned());
    }
    (list, set)
}

// ---- Save readers (read only) ----

/// Banjo-Kazooie's 4 Kbit EEPROM: 4 game files of 0x78 bytes (15 blocks of 8), each
/// `SaveData { magic 0x11, slotIndex 1..3, data[0x70], padding[2], checksum }`. Inside data
/// (offsets from the file's start, as savedata_init lays them out): jiggies bitfield at 0x02
/// (13 bytes), empty honeycombs at 0x0F (3), Mumbo Tokens at 0x12 (16), high note scores at
/// 0x22 (u64, nine 7-bit values). The best of the three game slots counts.
pub fn read_bk_eeprom(bytes: &[u8]) -> BTreeMap<String, u64> {
    const FILE: usize = 0x78;
    let mut best: BTreeMap<String, u64> = BTreeMap::new();
    for f in 0..4 {
        let Some(file) = bytes.get(f * FILE..(f + 1) * FILE) else { break };
        if file[0] != 0x11 || !(1..=3).contains(&file[1]) {
            continue;
        }
        let bits = |range: std::ops::Range<usize>| file[range].iter().map(|b| b.count_ones() as u64).sum::<u64>();
        let packed = u64::from_be_bytes(file[0x22..0x2A].try_into().unwrap());
        let notes: u64 = (0..9).map(|i| (packed >> (7 * i)) & 0x7F).map(|n| n.min(100)).sum();
        for (k, v) in [
            ("jiggies", bits(0x02..0x0F).min(100)),
            ("honeycombs", bits(0x0F..0x12).min(24)),
            ("mumbo_tokens", bits(0x12..0x22).min(116)),
            ("notes", notes),
        ] {
            let e = best.entry(k.to_string()).or_insert(0);
            *e = (*e).max(v);
        }
    }
    best
}

pub fn read_save(reader: &str, path: &Path) -> BTreeMap<String, u64> {
    let Ok(bytes) = std::fs::read(path) else { return BTreeMap::new() };
    match reader {
        "bk_eeprom" => read_bk_eeprom(&bytes),
        _ => BTreeMap::new(),
    }
}

// ---- Unlocks, per profile ----

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Unlocks {
    /// "<game>:<achievement>" -> unix time unlocked.
    #[serde(default)]
    pub unlocked: BTreeMap<String, u64>,
}

pub fn key(game: &str, id: &str) -> String {
    format!("{game}:{id}")
}

impl Unlocks {
    pub fn load(dir: &Path, profile: &str) -> Self {
        std::fs::read(dir.join(format!("{profile}.json"))).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path, profile: &str) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let path = dir.join(format!("{profile}.json"));
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }
}

/// What the page shows for one achievement.
#[derive(Serialize, Clone, Debug)]
pub struct View {
    pub key: String,
    pub game: String,
    pub id: String,
    pub title: String,
    pub description: String,
    pub points: u32,
    pub kind: String,
    pub badge: String,
    pub hidden: bool,
    pub unlocked_at: Option<u64>,
    /// Current value and goal, for a progress bar.
    pub value: u64,
    pub goal: u64,
}

/// Unlocks every achievement of `game` whose trigger is met by `stats`. Returns the views of
/// all its achievements and the ones unlocked now.
pub fn evaluate(defs: &Defs, game: &str, stats: &BTreeMap<String, u64>, unlocks: &mut Unlocks, now: u64) -> (Vec<View>, Vec<View>) {
    let (list, _) = for_game(defs, game);
    let mut all = Vec::new();
    let mut new = Vec::new();
    for a in list {
        let k = key(game, &a.id);
        let value = stats.get(&a.trigger.stat).copied().unwrap_or(0);
        let mut unlocked_at = unlocks.unlocked.get(&k).copied();
        if unlocked_at.is_none() && value >= a.trigger.at_least {
            unlocks.unlocked.insert(k.clone(), now);
            unlocked_at = Some(now);
        }
        let view = View {
            key: k,
            game: game.to_string(),
            id: a.id.clone(),
            title: a.title.clone(),
            description: a.description.clone(),
            points: a.points,
            kind: a.kind.clone(),
            badge: a.badge.clone(),
            hidden: a.hidden,
            unlocked_at,
            value: value.min(a.trigger.at_least),
            goal: a.trigger.at_least,
        };
        if unlocked_at == Some(now) && !stats.is_empty() {
            new.push(view.clone());
        }
        all.push(view);
    }
    (all, new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_definitions_are_valid() {
        let d = parse(BUILTIN).unwrap();
        assert!(d.games.contains_key("banjo"));
        assert!(parse(&BUILTIN.replace("\"jiggies\", \"at_least\": 1 }", "\"inventado\", \"at_least\": 1 }")).is_err());
    }

    fn bk_file(slot: u8, jiggy_bytes: &[u8], notes: &[u64]) -> Vec<u8> {
        let mut f = vec![0u8; 0x78];
        f[0] = 0x11;
        f[1] = slot;
        f[2..2 + jiggy_bytes.len()].copy_from_slice(jiggy_bytes);
        let mut packed = 0u64;
        for n in notes {
            packed = (packed << 7) | n;
        }
        f[0x22..0x2A].copy_from_slice(&packed.to_be_bytes());
        f
    }

    #[test]
    fn banjo_save_counts_the_best_slot() {
        let mut eeprom = bk_file(1, &[0xFF, 0x03], &[100, 50, 0, 0, 0, 0, 0, 0, 7]);
        eeprom.extend(bk_file(2, &[0x01], &[]));
        eeprom.extend(vec![0u8; 0x78]); // empty file
        eeprom.extend(vec![0u8; 0x78 + 0x20]);
        let s = read_bk_eeprom(&eeprom);
        assert_eq!(s["jiggies"], 10);
        assert_eq!(s["notes"], 157);
        assert_eq!(read_bk_eeprom(&[0u8; 512]).get("jiggies"), None, "sem save, nada");
    }

    #[test]
    fn unlocks_happen_once_and_keep_their_date() {
        let d = parse(BUILTIN).unwrap();
        let mut u = Unlocks::default();
        let stats: BTreeMap<String, u64> = [("jiggies".to_string(), 12), ("play_count".to_string(), 1)].into();
        let (all, new) = evaluate(&d, "banjo", &stats, &mut u, 100);
        assert_eq!(new.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), vec!["primeira-sessao", "jiggy-1", "jiggy-10"]);
        assert_eq!(all.iter().find(|v| v.id == "jiggy-50").unwrap().value, 12);
        let (_, again) = evaluate(&d, "banjo", &stats, &mut u, 200);
        assert!(again.is_empty());
        assert_eq!(u.unlocked["banjo:jiggy-1"], 100);
    }
}

//! Play history, as Playnite's GameActivity: one record per session (when it started, when
//! it ended and how much of it counted as play time), kept in history.json apart from the
//! library so the library stays small. The statistics page is computed from it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Shorter sessions (a crash on start, a launcher that closed) are not history.
pub const MIN_SESSION_SECS: u64 = 10;
/// The oldest records are dropped past this, so the file never grows without end.
const MAX_RECORDS: usize = 20_000;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Record {
    pub id: String,
    /// Unix times.
    pub start: u64,
    pub end: u64,
    /// Counted play time (time asleep is left out), so it can be less than end - start.
    pub seconds: u64,
}

#[derive(Serialize, Deserialize, Default)]
pub struct History {
    #[serde(default)]
    pub sessions: Vec<Record>,
}

impl History {
    pub fn load(path: &Path) -> Self {
        std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    /// Returns false when the session was too short to keep.
    pub fn record(&mut self, r: Record) -> bool {
        if r.seconds < MIN_SESSION_SECS {
            return false;
        }
        self.sessions.push(r);
        if self.sessions.len() > MAX_RECORDS {
            let extra = self.sessions.len() - MAX_RECORDS;
            self.sessions.drain(0..extra);
        }
        true
    }

    /// The game's sessions, newest first.
    pub fn of(&self, id: &str, max: usize) -> Vec<Record> {
        self.sessions.iter().rev().filter(|r| r.id == id).take(max).cloned().collect()
    }
}

/// Day number in the player's time zone (`offset` = seconds east of UTC).
fn day(unix: u64, offset: i64) -> i64 {
    (unix as i64 + offset).div_euclid(86_400)
}

/// The Monday of that day's week (1970-01-01 was a Thursday).
fn week_start(day: i64) -> i64 {
    day - (day + 3).rem_euclid(7)
}

#[derive(Serialize, Default, Debug, PartialEq)]
pub struct Bucket {
    /// Day number (days since 1970-01-01 in the player's time zone) of the first day.
    pub day: i64,
    pub seconds: u64,
}

#[derive(Serialize, Default, Debug)]
pub struct GameStats {
    pub id: String,
    pub seconds: u64,
    pub sessions: u32,
    pub last: u64,
}

#[derive(Serialize, Default, Debug)]
pub struct Stats {
    pub sessions: u32,
    pub seconds: u64,
    /// The last `weeks` weeks, oldest first, this week last.
    pub weeks: Vec<Bucket>,
    /// The last 14 days, oldest first, today last.
    pub days: Vec<Bucket>,
    /// Most played in the history first.
    pub games: Vec<GameStats>,
    pub longest: Option<Record>,
    /// Day numbers with at least one session (for "dias seguidos").
    pub streak_days: u32,
}

pub fn stats(history: &History, now: u64, offset: i64, weeks: usize) -> Stats {
    let today = day(now, offset);
    let this_week = week_start(today);
    let mut out = Stats {
        weeks: (0..weeks as i64).rev().map(|w| Bucket { day: this_week - w * 7, seconds: 0 }).collect(),
        days: (0..14).rev().map(|d| Bucket { day: today - d, seconds: 0 }).collect(),
        ..Default::default()
    };
    let mut games: BTreeMap<&str, GameStats> = BTreeMap::new();
    let mut played_days = std::collections::BTreeSet::new();
    for r in &history.sessions {
        out.sessions += 1;
        out.seconds += r.seconds;
        let d = day(r.start, offset);
        played_days.insert(d);
        let w = week_start(d);
        if let Some(b) = out.weeks.iter_mut().find(|b| b.day == w) {
            b.seconds += r.seconds;
        }
        if let Some(b) = out.days.iter_mut().find(|b| b.day == d) {
            b.seconds += r.seconds;
        }
        let g = games.entry(&r.id).or_insert_with(|| GameStats { id: r.id.clone(), ..Default::default() });
        g.seconds += r.seconds;
        g.sessions += 1;
        g.last = g.last.max(r.end);
        if out.longest.as_ref().map(|l| r.seconds > l.seconds).unwrap_or(true) {
            out.longest = Some(r.clone());
        }
    }
    // Days in a row ending today (or yesterday, when today has nothing yet).
    let mut d = if played_days.contains(&today) { today } else { today - 1 };
    while played_days.contains(&d) {
        out.streak_days += 1;
        d -= 1;
    }
    out.games = games.into_values().collect();
    out.games.sort_by(|a, b| b.seconds.cmp(&a.seconds));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, start: u64, seconds: u64) -> Record {
        Record { id: id.into(), start, end: start + seconds, seconds }
    }

    #[test]
    fn short_sessions_are_not_kept() {
        let mut h = History::default();
        assert!(!h.record(rec("a", 0, 5)));
        assert!(h.record(rec("a", 0, 60)));
        assert_eq!(h.sessions.len(), 1);
    }

    #[test]
    fn weeks_start_on_monday_in_the_players_time_zone() {
        // 2026-10-19 is a Monday. 01:00 UTC on Monday is still Sunday 22:00 in Brasília (-3 h).
        let monday_0100_utc = 1_792_371_600;
        assert_eq!(week_start(day(monday_0100_utc, 0)), day(monday_0100_utc, 0));
        assert_eq!(week_start(day(monday_0100_utc, -3 * 3600)), day(monday_0100_utc, 0) - 7);
    }

    #[test]
    fn stats_add_up_by_game_week_day_and_streak() {
        let now = 1_792_371_600 + 12 * 3600; // Monday 2026-10-19, 13:00 UTC
        let mut h = History::default();
        h.record(rec("banjo", now - 3600, 1800));
        h.record(rec("banjo", now - 86_400, 600));
        h.record(rec("gow3", now - 2 * 86_400, 7200));
        h.record(rec("gow3", now - 40 * 86_400, 100)); // outside the 14 days, inside 12 weeks
        let s = stats(&h, now, 0, 12);
        assert_eq!(s.sessions, 4);
        assert_eq!(s.seconds, 9700);
        assert_eq!(s.games[0].id, "gow3");
        assert_eq!(s.games[0].seconds, 7300);
        assert_eq!(s.days.last().unwrap().seconds, 1800, "hoje");
        assert_eq!(s.weeks.last().unwrap().seconds, 1800, "esta semana começou hoje (segunda)");
        assert_eq!(s.weeks.iter().map(|w| w.seconds).sum::<u64>(), 9700);
        assert_eq!(s.streak_days, 3);
        assert_eq!(s.longest.unwrap().seconds, 7200);
    }
}

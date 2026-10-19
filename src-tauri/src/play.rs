//! Starting ports and timing sessions. Like Hydra's process watcher, a background thread
//! looks every few seconds for programs running from each game's folder, so a game counts
//! whether RecompHUB opened it or not. A port can hand over to other programs in its folder
//! (gow3_pc starts bin\gow3-probe.exe); the session lasts while any of them runs. Play time
//! is written every 30 seconds, so a crash or closing RecompHUB loses little.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tauri::{AppHandle, Emitter, Manager};

use crate::library::now;
use crate::AppState;

const POLL: Duration = Duration::from_secs(3);
const SAVE_EVERY: Duration = Duration::from_secs(30);
/// A longer gap between two looks means the PC was asleep: not play time.
const MAX_TICK: Duration = Duration::from_secs(30);
/// How long a port may take to show up after Play before "Abrindo…" is dropped.
const START_TIMEOUT: Duration = Duration::from_secs(60);

pub struct Session {
    /// Unix time the port was first seen running.
    pub started: u64,
    last_tick: Instant,
    /// Play time not yet written to the library.
    unsaved: Duration,
}

#[derive(Default)]
pub struct Sessions {
    pub running: HashMap<String, Session>,
    launching: HashMap<String, Instant>,
}

impl Sessions {
    pub fn is_running(&self, id: &str) -> bool {
        self.running.contains_key(id)
    }

    pub fn is_launching(&self, id: &str) -> bool {
        self.launching.get(id).map(|t| t.elapsed() < START_TIMEOUT).unwrap_or(false)
    }

    pub fn started(&self, id: &str) -> Option<u64> {
        self.running.get(id).map(|s| s.started)
    }

    /// Whole seconds of unsaved play time of each session with at least `min` unsaved.
    fn take_unsaved(&mut self, min: Duration) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        for (id, s) in self.running.iter_mut() {
            if s.unsaved >= min && s.unsaved.as_secs() > 0 {
                let secs = s.unsaved.as_secs();
                s.unsaved -= Duration::from_secs(secs);
                out.push((id.clone(), secs));
            }
        }
        out
    }
}

fn folder_prefix(dir: &Path) -> String {
    let mut s = dir.to_string_lossy().replace('/', "\\").to_lowercase();
    if !s.ends_with('\\') {
        s.push('\\');
    }
    s
}

fn refresh(sys: &mut System) {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet),
    );
}

/// Processes whose program is inside the folder `prefix`.
fn processes_in(sys: &System, prefix: &str) -> Vec<Pid> {
    sys.processes()
        .iter()
        .filter(|(_, p)| {
            p.exe()
                .map(|e| e.to_string_lossy().replace('/', "\\").to_lowercase().starts_with(prefix))
                .unwrap_or(false)
        })
        .map(|(pid, _)| *pid)
        .collect()
}

/// Adds play time to the library and saves it.
fn credit(app: &AppHandle, credits: &[(String, u64)], ended: &[String]) {
    if credits.is_empty() && ended.is_empty() {
        return;
    }
    let state = app.state::<AppState>();
    let mut lib = state.library.lock().unwrap();
    let t = now();
    for (id, secs) in credits {
        if let Some(e) = lib.games.get_mut(id) {
            e.total_seconds += secs;
            e.last_played = Some(t);
        }
    }
    for id in ended {
        if let Some(e) = lib.games.get_mut(id) {
            e.last_played = Some(t);
        }
    }
    let _ = lib.save(&state.library_path);
}

pub fn spawn_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        let mut sys = System::new();
        loop {
            refresh(&mut sys);
            tick(&app, &sys);
            std::thread::sleep(POLL);
        }
    });
}

fn tick(app: &AppHandle, sys: &System) {
    let state = app.state::<AppState>();
    let folders: Vec<(String, String)> = state
        .library
        .lock()
        .unwrap()
        .games
        .iter()
        .map(|(id, e)| (id.clone(), folder_prefix(Path::new(&e.install_dir))))
        .collect();

    let at = Instant::now();
    let mut started = Vec::new();
    let mut ended = Vec::new();
    let credits = {
        let mut guard = state.sessions.lock().unwrap();
        let s = &mut *guard;
        for (id, prefix) in &folders {
            let running = !processes_in(sys, prefix).is_empty();
            match (s.running.get_mut(id), running) {
                (None, true) => {
                    s.launching.remove(id);
                    s.running.insert(id.clone(), Session { started: now(), last_tick: at, unsaved: Duration::ZERO });
                    started.push(id.clone());
                }
                (Some(sess), _) => {
                    let gap = at - sess.last_tick;
                    if gap <= MAX_TICK {
                        sess.unsaved += gap;
                    }
                    sess.last_tick = at;
                    if !running {
                        ended.push((id.clone(), now().saturating_sub(sess.started)));
                    }
                }
                (None, false) => {}
            }
        }
        s.launching.retain(|_, t| t.elapsed() < START_TIMEOUT);
        let mut credits = s.take_unsaved(SAVE_EVERY);
        for (id, _) in &ended {
            if let Some(sess) = s.running.remove(id) {
                let secs = sess.unsaved.as_secs();
                if secs > 0 {
                    credits.push((id.clone(), secs));
                }
            }
        }
        credits
    };

    let ended_ids: Vec<String> = ended.iter().map(|(id, _)| id.clone()).collect();
    credit(app, &credits, &ended_ids);
    for id in started {
        let _ = app.emit("session", serde_json::json!({ "id": id, "state": "running" }));
    }
    for (id, seconds) in ended {
        let _ = app.emit("session", serde_json::json!({ "id": id, "state": "ended", "seconds": seconds }));
    }
}

/// Writes the play time still in memory (RecompHUB is closing).
pub fn flush(app: &AppHandle) {
    let credits = app.state::<AppState>().sessions.lock().unwrap().take_unsaved(Duration::ZERO);
    credit(app, &credits, &[]);
}

pub fn start(app: &AppHandle, id: &str, install_dir: &Path, exe: &str) -> Result<(), String> {
    let exe_path = install_dir.join(exe);
    if !exe_path.is_file() {
        return Err(format!("Não encontrei {} na pasta do jogo.", exe));
    }
    std::process::Command::new(&exe_path)
        .current_dir(install_dir)
        .spawn()
        .map_err(|e| format!("O Windows não abriu o jogo: {e}"))?;
    app.state::<AppState>().sessions.lock().unwrap().launching.insert(id.to_string(), Instant::now());
    let _ = app.emit("session", serde_json::json!({ "id": id, "state": "starting" }));
    Ok(())
}

/// Closes every program running from the game's folder ("Fechar jogo").
pub fn stop(install_dir: &Path) -> Result<usize, String> {
    let mut sys = System::new();
    refresh(&mut sys);
    let pids = processes_in(&sys, &folder_prefix(install_dir));
    let mut closed = 0;
    for pid in &pids {
        if let Some(p) = sys.process(*pid) {
            if p.kill() {
                closed += 1;
            }
        }
    }
    if pids.is_empty() {
        return Err("O jogo não está aberto.".into());
    }
    Ok(closed)
}

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

use crate::history::{Record, MIN_SESSION_SECS};
use crate::library::{now, Completion};
use crate::AppState;

const POLL: Duration = Duration::from_secs(3);
const SAVE_EVERY: Duration = Duration::from_secs(30);
/// A longer gap between two looks means the PC was asleep: not play time.
const MAX_TICK: Duration = Duration::from_secs(30);
/// How long a port may take to show up after Play before "Abrindo…" is dropped.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// The save folder of a running game is looked at every 5 looks (15 seconds).
const SAVE_CHECK_EVERY: u32 = 5;

pub struct Session {
    /// Unix time the port was first seen running.
    pub started: u64,
    last_tick: Instant,
    /// Play time not yet written to the library.
    unsaved: Duration,
    /// All play time of this session, for the history.
    counted: Duration,
    /// Looks since the session started; the save folder is checked every SAVE_CHECK_EVERY.
    ticks: u32,
    /// Newest change seen in the save folder.
    last_save: Option<u64>,
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

/// Processes whose program is inside the folder `prefix`. A game running as administrator
/// may not show its path to RecompHUB; then its program name (`exe_name`) is used.
fn processes_in(sys: &System, prefix: &str, exe_name: Option<&str>) -> Vec<Pid> {
    sys.processes()
        .iter()
        .filter(|(_, p)| match p.exe() {
            Some(e) => e.to_string_lossy().replace('/', "\\").to_lowercase().starts_with(prefix),
            None => exe_name.map(|n| p.name().to_string_lossy().eq_ignore_ascii_case(n)).unwrap_or(false),
        })
        .map(|(pid, _)| *pid)
        .collect()
}

/// The program's file name ("BanjoRecompiled.exe" from "bin\\BanjoRecompiled.exe").
pub fn exe_name(exe: &str) -> Option<String> {
    Path::new(&exe.replace('\\', "/")).file_name().map(|n| n.to_string_lossy().to_string()).filter(|n| !n.is_empty())
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
    let folders: Vec<(String, String, Option<String>)> = state
        .library
        .lock()
        .unwrap()
        .games
        .iter()
        .map(|(id, e)| (id.clone(), folder_prefix(Path::new(&e.install_dir)), crate::program_of(id, e).and_then(|x| exe_name(&x))))
        .collect();

    let at = Instant::now();
    let mut started = Vec::new();
    let mut ended = Vec::new();
    let mut records = Vec::new();
    let mut check_saves = Vec::new();
    let profile = state.profiles.lock().unwrap().active.clone();
    let credits = {
        let mut guard = state.sessions.lock().unwrap();
        let s = &mut *guard;
        for (id, prefix, name) in &folders {
            let running = !processes_in(sys, prefix, name.as_deref()).is_empty();
            match (s.running.get_mut(id), running) {
                (None, true) => {
                    s.launching.remove(id);
                    s.running.insert(id.clone(), Session { started: now(), last_tick: at, unsaved: Duration::ZERO, counted: Duration::ZERO, ticks: 0, last_save: None });
                    started.push(id.clone());
                }
                (Some(sess), _) => {
                    let gap = at - sess.last_tick;
                    if gap <= MAX_TICK {
                        sess.unsaved += gap;
                        sess.counted += gap;
                    }
                    sess.last_tick = at;
                    sess.ticks += 1;
                    if running && (sess.ticks == 1 || sess.ticks % SAVE_CHECK_EVERY == 0) {
                        check_saves.push(id.clone());
                    }
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
                records.push(record(id, &sess, &profile));
            }
        }
        credits
    };

    let ended_ids: Vec<String> = ended.iter().map(|(id, _)| id.clone()).collect();
    credit(app, &credits, &ended_ids);
    remember(app, &started, records);
    for (id, _) in &ended {
        crate::backup_after_session(app.clone(), id.clone());
        crate::check_achievements(app, id, true);
    }
    watch_saves(app, &check_saves);
    if !started.is_empty() || !ended.is_empty() {
        crate::tray::refresh(app);
    }
    for id in started {
        let _ = app.emit("session", serde_json::json!({ "id": id, "state": "running" }));
    }
    for (id, seconds) in ended {
        let _ = app.emit("session", serde_json::json!({ "id": id, "state": "ended", "seconds": seconds }));
    }
}

/// The save watcher (as FileWatcher, but by looking, since save folders are small): when the
/// newest file in a running game's save folder changes, "save-changed" goes to the page
/// (and, later, to the achievements).
fn watch_saves(app: &AppHandle, ids: &[String]) {
    let state = app.state::<AppState>();
    for id in ids {
        let Some(newest) = crate::autosave::newest_change(&crate::save_sources(&state, id)) else { continue };
        let changed = {
            let mut s = state.sessions.lock().unwrap();
            match s.running.get_mut(id) {
                Some(sess) => {
                    let before = sess.last_save.replace(newest);
                    before.is_some() && before != Some(newest)
                }
                None => false,
            }
        };
        if changed {
            let _ = app.emit("save-changed", serde_json::json!({ "id": id, "at": newest }));
            crate::check_achievements(app, id, true);
        }
    }
}

/// Writes the play time still in memory (RecompHUB is closing). Sessions still running go
/// into the history as they are; if the game keeps running, the next start begins a new one.
pub fn flush(app: &AppHandle) {
    let (credits, records) = {
        let state = app.state::<AppState>();
        let mut s = state.sessions.lock().unwrap();
        let credits = s.take_unsaved(Duration::ZERO);
        let profile = state.profiles.lock().unwrap().active.clone();
        let records: Vec<Record> = s.running.iter().map(|(id, sess)| record(id, sess, &profile)).collect();
        s.running.clear();
        (credits, records)
    };
    credit(app, &credits, &[]);
    remember(app, &[], records);
}

fn record(id: &str, sess: &Session, profile: &str) -> Record {
    Record { id: id.to_string(), start: sess.started, end: now(), seconds: sess.counted.as_secs(), profile: profile.to_string() }
}

/// History of finished sessions, the play count, and "Jogando" for a game played for the
/// first time (as Playnite does).
fn remember(app: &AppHandle, started: &[String], records: Vec<Record>) {
    if started.is_empty() && records.is_empty() {
        return;
    }
    let state = app.state::<AppState>();
    {
        let mut lib = state.library.lock().unwrap();
        for id in started {
            if let Some(e) = lib.games.get_mut(id) {
                if matches!(e.completion, Completion::None | Completion::NotPlayed) {
                    e.completion = Completion::Playing;
                }
            }
        }
        for r in records.iter().filter(|r| r.seconds >= MIN_SESSION_SECS) {
            if let Some(e) = lib.games.get_mut(&r.id) {
                e.play_count += 1;
            }
        }
        let _ = lib.save(&state.library_path);
    }
    if records.is_empty() {
        return;
    }
    let mut history = state.history.lock().unwrap();
    for r in records {
        history.record(r);
    }
    let _ = history.save(&state.history_path);
}

/// Splits "--fullscreen --rom \"C:\\My Games\\x.z64\"" into arguments: spaces separate,
/// double quotes group, and a quote is never passed on.
pub fn split_args(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut has = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

/// A name Windows accepts for an environment variable.
pub fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && name.len() <= 64
}

/// Windows' "The requested operation requires elevation": the program's manifest asks for
/// administrator, and CreateProcess never shows the UAC prompt by itself.
const ERROR_ELEVATION_REQUIRED: i32 = 740;
/// The player said no on the UAC prompt.
const ERROR_CANCELLED: i32 = 1223;

/// How the game was opened.
#[derive(PartialEq, Debug)]
pub enum Started {
    Normal,
    /// Through the administrator prompt.
    Admin,
}

/// Arguments back into one command line, quoting the ones with spaces.
pub fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            let a = a.replace('"', "");
            if a.is_empty() || a.contains(' ') || a.contains('\t') {
                format!("\"{a}\"")
            } else {
                a
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// ShellExecuteEx with the "runas" verb: Windows shows the UAC prompt and starts the program
/// as administrator. Environment variables cannot be passed this way.
fn start_as_admin(exe_path: &Path, dir: &Path, args: &[String]) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(exe_path.as_os_str());
    let params = wide(std::ffi::OsStr::new(&join_args(args)));
    let directory = wide(dir.as_os_str());
    // SAFETY: every pointer is to a NUL-terminated buffer that lives until the call returns.
    let ok = unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.lpDirectory = directory.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        ShellExecuteExW(&mut info) != 0
    };
    if ok {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    if err.raw_os_error() == Some(ERROR_CANCELLED) {
        Err("Você não deu a permissão de administrador, então o jogo não abriu.".into())
    } else {
        Err(format!("O Windows não abriu o jogo como administrador: {err}"))
    }
}

pub fn start(
    app: &AppHandle,
    id: &str,
    install_dir: &Path,
    exe: &str,
    args: &str,
    env: &[crate::library::EnvVar],
    as_admin: bool,
) -> Result<Started, String> {
    let exe_path = install_dir.join(exe);
    if !exe_path.is_file() {
        return Err(format!("Não encontrei {} na pasta do jogo.", exe));
    }
    let args = split_args(args);
    let started = if as_admin {
        start_as_admin(&exe_path, install_dir, &args)?;
        Started::Admin
    } else {
        let mut cmd = std::process::Command::new(&exe_path);
        cmd.current_dir(install_dir).args(&args);
        for v in env.iter().filter(|v| valid_env_name(&v.name)) {
            cmd.env(&v.name, &v.value);
        }
        match cmd.spawn() {
            Ok(_) => Started::Normal,
            // The program asks for administrator: ask Windows for it (UAC) instead of failing.
            Err(e) if e.raw_os_error() == Some(ERROR_ELEVATION_REQUIRED) => {
                start_as_admin(&exe_path, install_dir, &args)?;
                Started::Admin
            }
            Err(e) => return Err(format!("O Windows não abriu o jogo: {e}")),
        }
    };
    app.state::<AppState>().sessions.lock().unwrap().launching.insert(id.to_string(), Instant::now());
    let _ = app.emit("session", serde_json::json!({ "id": id, "state": "starting" }));
    Ok(started)
}

/// Closes every program running from the game's folder ("Fechar jogo").
pub fn stop(install_dir: &Path, exe_name: Option<&str>) -> Result<usize, String> {
    let mut sys = System::new();
    refresh(&mut sys);
    let pids = processes_in(&sys, &folder_prefix(install_dir), exe_name);
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
    if closed == 0 {
        return Err("O jogo está rodando como administrador e o Windows não deixa o RecompHUB fechá-lo. Feche pelo próprio jogo.".into());
    }
    Ok(closed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_keep_quoted_paths_together() {
        assert_eq!(split_args(""), Vec::<String>::new());
        assert_eq!(split_args("  --a   -b "), vec!["--a", "-b"]);
        assert_eq!(split_args(r#"--rom "C:\My Games\x.z64" -v"#), vec!["--rom", r"C:\My Games\x.z64", "-v"]);
        assert_eq!(split_args(r#"--name """#), vec!["--name", ""]);
    }

    #[test]
    fn arguments_go_back_into_one_line() {
        let args = split_args(r#"--rom "C:\My Games\x.z64" -v"#);
        assert_eq!(join_args(&args), r#"--rom "C:\My Games\x.z64" -v"#);
        assert_eq!(split_args(&join_args(&args)), args);
        assert_eq!(exe_name(r"bin\BanjoRecompiled.exe").as_deref(), Some("BanjoRecompiled.exe"));
    }

    #[test]
    fn environment_names() {
        assert!(valid_env_name("DXVK_HUD"));
        assert!(valid_env_name("_x1"));
        assert!(!valid_env_name("1ABC"));
        assert!(!valid_env_name("A=B"));
        assert!(!valid_env_name(""));
    }
}

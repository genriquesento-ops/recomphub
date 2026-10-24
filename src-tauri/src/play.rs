//! Starting ports and timing sessions. Like Hydra's process watcher, a background thread
//! looks every few seconds for programs running from each game's folder, so a game counts
//! whether RecompHUB opened it or not. A port can hand over to other programs in its folder
//! (gow3_pc starts bin\gow3-probe.exe); the session lasts while any of them runs. Play time
//! is written every 30 seconds, so a crash or closing RecompHUB loses little.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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
    /// Programs RecompHUB started, to read how they ended (diagnose.rs).
    children: HashMap<String, std::process::Child>,
    /// Crash code of the started program, kept until its session ends.
    exit_codes: HashMap<String, u32>,
    /// Closed by "Fechar jogo": not an error.
    stopped: HashSet<String>,
}

impl Sessions {
    pub fn is_running(&self, id: &str) -> bool {
        self.running.contains_key(id)
    }

    pub fn is_launching(&self, id: &str) -> bool {
        self.launching.get(id).map(|t| t.elapsed() < START_TIMEOUT).unwrap_or(false)
    }

    pub fn mark_stopped(&mut self, id: &str) {
        self.stopped.insert(id.to_string());
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
    // Started programs that died before their session was seen (a missing DLL ends the
    // program at once): (id, crash code, seconds).
    let mut failed = Vec::new();
    // Ended sessions to look at: (id, start, seconds, crash code, closed by "Fechar jogo").
    let mut checks = Vec::new();
    let credits = {
        let mut guard = state.sessions.lock().unwrap();
        let s = &mut *guard;
        let mut exited = Vec::new();
        for (id, child) in s.children.iter_mut() {
            if let Ok(Some(status)) = child.try_wait() {
                exited.push((id.clone(), status.code().map(|c| c as u32)));
            }
        }
        for (id, code) in exited {
            s.children.remove(&id);
            // Only a crash code counts: a launcher may quit with a code of its own after
            // handing over to the game.
            let Some(code) = code.filter(|c| crate::diagnose::is_crash_code(*c)) else { continue };
            if s.running.contains_key(&id) {
                s.exit_codes.insert(id, code);
            } else if let Some(t) = s.launching.remove(&id) {
                if !s.stopped.remove(&id) {
                    failed.push((id, code, t.elapsed().as_secs()));
                }
            }
        }
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
                s.children.remove(id);
                checks.push((id.clone(), sess.started, now().saturating_sub(sess.started), s.exit_codes.remove(id), s.stopped.remove(id)));
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
    for (id, start, seconds, code, stopped) in checks {
        crate::fps::stop(app, Some(&id));
        let app = app.clone();
        // Windows' error records and the drivers take a second to read; not on the watcher's
        // pace. The drivers are remembered only after a session that ended well, so "the
        // driver changed since it last worked" stays true after a crash.
        std::thread::spawn(move || {
            let crashed = !stopped && crate::check_crash(&app, &id, start, Some(seconds), code, false).is_some();
            if !crashed {
                crate::remember_shader_driver(&app, &id);
            }
        });
    }
    for (id, code, seconds) in failed {
        let app2 = app.clone();
        let id2 = id.clone();
        std::thread::spawn(move || {
            crate::check_crash(&app2, &id2, now().saturating_sub(seconds), Some(seconds), Some(code), false);
        });
        let _ = app.emit("session", serde_json::json!({ "id": id, "state": "ended", "seconds": 0 }));
    }
    for id in started {
        if let Some(name) = folders.iter().find(|(i, _, _)| *i == id).and_then(|(_, _, n)| n.clone()) {
            let (app, id) = (app.clone(), id.clone());
            // PresentMon is started away from the watcher, which must keep its pace.
            std::thread::spawn(move || crate::fps::start(&app, &id, &name));
        }
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
/// "The file contains a virus" / "...was deleted": the antivirus blocked or removed it.
const ERROR_VIRUS_INFECTED: i32 = 225;
const ERROR_VIRUS_DELETED: i32 = 226;

/// What to tell the player when the antivirus is the reason. Ports are new, unsigned programs,
/// and Windows Defender sometimes flags one (a false positive, as with Donkey Kong 64).
pub const ANTIVIRUS_HELP: &str = "O antivírus bloqueou ou apagou um arquivo do port. Com programas novos e sem assinatura isso pode ser alarme falso, mas nem sempre é. Veja o arquivo em Segurança do Windows > Proteção contra vírus e ameaças > Histórico de proteção. Só restaure se o port veio da página oficial do projeto pelo RecompHUB; na dúvida, pergunte no GitHub do projeto antes.";

pub fn antivirus_error(e: &std::io::Error) -> Option<String> {
    matches!(e.raw_os_error(), Some(ERROR_VIRUS_INFECTED) | Some(ERROR_VIRUS_DELETED)).then(|| ANTIVIRUS_HELP.to_string())
}

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

/// The first argument of an elevated copy of RecompHUB that only opens a game (see
/// `start_as_admin`).
pub const ELEVATED_FLAG: &str = "--abrir-jogo-como-admin";
/// Windows limits a command line to 32,767 characters.
const MAX_ELEVATED_PAYLOAD: usize = 30_000;

/// What the elevated copy opens. It travels on the command line, not in a file, so nothing can
/// change it between the UAC prompt and the launch.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
pub struct ElevatedLaunch {
    pub exe: PathBuf,
    pub dir: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub fn encode_launch(l: &ElevatedLaunch) -> Result<String, String> {
    use base64::Engine;
    // A path that is not valid Unicode cannot be written as JSON.
    let json = serde_json::to_vec(l).map_err(|_| "O caminho do jogo tem caracteres que o RecompHUB não consegue repassar.".to_string())?;
    // URL-safe base64 has no spaces or quotes, so it is one plain argument.
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
}

pub fn decode_launch(payload: &str) -> Option<ElevatedLaunch> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Runs in the elevated copy, before any window opens: starts the game with its variables
/// (already as administrator, so no second prompt) and returns.
pub fn run_elevated_launch(payload: &str) -> Result<(), String> {
    let l = decode_launch(payload).ok_or("Pedido de abertura inválido.")?;
    let is_exe = l.exe.extension().map(|e| e.eq_ignore_ascii_case("exe")).unwrap_or(false);
    if !is_exe || !l.exe.is_file() {
        return Err(format!("Não encontrei {}.", l.exe.display()));
    }
    if l.env.iter().any(|(k, _)| !valid_env_name(k)) {
        return Err("Nome de variável de ambiente inválido.".into());
    }
    std::process::Command::new(&l.exe)
        .current_dir(&l.dir)
        .args(&l.args)
        .envs(l.env.iter().map(|(k, v)| (k, v)))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("O Windows não abriu o jogo: {e}"))
}

/// An error from the elevated copy, which has no window of its own.
pub fn show_error(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(message), wide("RecompHUB"));
    // SAFETY: both strings are NUL-terminated and live until the call returns.
    unsafe {
        MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

/// ShellExecuteEx with the "runas" verb: Windows shows the UAC prompt and starts the program
/// as administrator. A program started this way does not get RecompHUB's environment, so when
/// the game has variables (Opções de inicialização) the elevated program is a copy of
/// RecompHUB that opens the game with them.
fn start_as_admin(exe_path: &Path, dir: &Path, args: &[String], env: &[(String, String)]) -> Result<(), String> {
    if env.is_empty() {
        return shell_runas(exe_path, dir, &join_args(args));
    }
    let payload = encode_launch(&ElevatedLaunch {
        exe: exe_path.to_path_buf(),
        dir: dir.to_path_buf(),
        args: args.to_vec(),
        env: env.to_vec(),
    })?;
    if payload.len() > MAX_ELEVATED_PAYLOAD {
        return Err("As variáveis de ambiente deste jogo são longas demais para abrir como administrador. Encurte-as em Opções de inicialização.".into());
    }
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    shell_runas(&me, dir, &format!("{ELEVATED_FLAG} {payload}"))
}

fn shell_runas(exe_path: &Path, dir: &Path, params: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(exe_path.as_os_str());
    let params = wide(std::ffi::OsStr::new(params));
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

/// Starts `exe_path` as administrator (one UAC prompt) and waits for it to finish: the
/// installers of "Antes de jogar". Returns its exit code.
pub fn runas_and_wait(exe_path: &Path, params: &str) -> Result<u32, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(exe_path.as_os_str());
    let params = wide(std::ffi::OsStr::new(params));
    // SAFETY: every pointer is to a NUL-terminated buffer that lives until the call returns;
    // the process handle is closed after use.
    let (ok, process) = unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        let ok = ShellExecuteExW(&mut info) != 0;
        (ok, info.hProcess)
    };
    if !ok {
        let err = std::io::Error::last_os_error();
        return Err(if err.raw_os_error() == Some(ERROR_CANCELLED) {
            "Você não deu a permissão de administrador, então nada foi instalado.".into()
        } else {
            format!("O Windows não abriu o instalador: {err}")
        });
    }
    if process.is_null() {
        return Err("O Windows abriu o instalador, mas não deixou o RecompHUB acompanhar. Confira em Configurações > Aplicativos se ele foi instalado.".into());
    }
    let mut code: u32 = 0;
    // SAFETY: `process` is a valid handle from ShellExecuteExW, closed once.
    unsafe {
        WaitForSingleObject(process, INFINITE);
        GetExitCodeProcess(process, &mut code);
        CloseHandle(process);
    }
    Ok(code)
}

/// Brings a running game's window to the front (restoring it when minimized). False when no
/// window of the game's processes was found.
pub fn focus(install_dir: &Path, exe_name: Option<&str>) -> bool {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE,
    };
    struct Search {
        pids: Vec<u32>,
        found: HWND,
    }
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let s = &mut *(lparam as *mut Search);
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if s.pids.contains(&pid) && IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() {
            s.found = hwnd;
            return 0;
        }
        1
    }
    let mut sys = System::new();
    refresh(&mut sys);
    let pids: Vec<u32> = processes_in(&sys, &folder_prefix(install_dir), exe_name).iter().map(|p| p.as_u32()).collect();
    if pids.is_empty() {
        return false;
    }
    let mut search = Search { pids, found: std::ptr::null_mut() };
    // SAFETY: `search` outlives EnumWindows, which calls `each` synchronously.
    unsafe {
        EnumWindows(Some(each), &mut search as *mut Search as LPARAM);
        if search.found.is_null() {
            return false;
        }
        if IsIconic(search.found) != 0 {
            ShowWindow(search.found, SW_RESTORE);
        }
        SetForegroundWindow(search.found) != 0
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
        // The folder is there but the program is not: usually the antivirus took it.
        return Err(if install_dir.is_dir() {
            format!("Não encontrei {} na pasta do jogo. {ANTIVIRUS_HELP}", exe)
        } else {
            format!("Não encontrei {} na pasta do jogo.", exe)
        });
    }
    let args = split_args(args);
    let env: Vec<(String, String)> =
        env.iter().filter(|v| valid_env_name(&v.name)).map(|v| (v.name.clone(), v.value.clone())).collect();
    let mut child = None;
    let started = if as_admin {
        start_as_admin(&exe_path, install_dir, &args, &env)?;
        Started::Admin
    } else {
        let mut cmd = std::process::Command::new(&exe_path);
        cmd.current_dir(install_dir).args(&args);
        cmd.envs(env.iter().map(|(k, v)| (k, v)));
        match cmd.spawn() {
            Ok(c) => {
                child = Some(c);
                Started::Normal
            }
            // The program asks for administrator: ask Windows for it (UAC) instead of failing.
            Err(e) if e.raw_os_error() == Some(ERROR_ELEVATION_REQUIRED) => {
                start_as_admin(&exe_path, install_dir, &args, &env)?;
                Started::Admin
            }
            Err(e) => return Err(antivirus_error(&e).unwrap_or_else(|| format!("O Windows não abriu o jogo: {e}"))),
        }
    };
    {
        let state = app.state::<AppState>();
        let mut s = state.sessions.lock().unwrap();
        s.launching.insert(id.to_string(), Instant::now());
        s.stopped.remove(id);
        s.exit_codes.remove(id);
        match child {
            Some(c) => {
                s.children.insert(id.to_string(), c);
            }
            None => {
                s.children.remove(id);
            }
        }
    }
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

    #[test]
    fn elevated_launch_survives_the_command_line() {
        let l = super::ElevatedLaunch {
            exe: r"C:\Jogos\Meu Jogo\jogo.exe".into(),
            dir: r"C:\Jogos\Meu Jogo".into(),
            args: vec!["--fullscreen".into(), "com espaço".into()],
            env: vec![("VK_DRIVER_FILES".into(), r#"C:\Mesa\lvp_icd.x86_64.json"#.into()), ("MESA_X".into(), "a \"b\" c=d".into())],
        };
        let p = super::encode_launch(&l).unwrap();
        assert!(p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_eq!(super::decode_launch(&p), Some(l));
        assert_eq!(super::decode_launch("não é base64"), None);
        assert!(super::run_elevated_launch(&p).unwrap_err().contains("Não encontrei"));
    }
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

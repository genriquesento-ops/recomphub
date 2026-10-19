//! Starting a port and timing the session. A port can hand over to other programs in its
//! folder (gow3_pc starts bin\gow3-probe.exe), so the session lasts while any program from
//! the port's folder is running.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tauri::{AppHandle, Emitter, Manager};

use crate::library::now;
use crate::AppState;

/// How long the port may take to show up before the session is dropped.
const START_TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_secs(2);

fn folder_prefix(dir: &Path) -> String {
    let mut s = dir.to_string_lossy().replace('/', "\\").to_lowercase();
    if !s.ends_with('\\') {
        s.push('\\');
    }
    s
}

fn port_running(sys: &mut System, prefix: &str) -> bool {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet),
    );
    sys.processes().values().any(|p| {
        p.exe()
            .map(|e| e.to_string_lossy().replace('/', "\\").to_lowercase().starts_with(prefix))
            .unwrap_or(false)
    })
}

pub fn start(app: AppHandle, id: String, install_dir: PathBuf, exe: &str) -> Result<(), String> {
    let exe_path = install_dir.join(exe);
    if !exe_path.is_file() {
        return Err(format!("Não encontrei {} na pasta do jogo.", exe));
    }
    std::process::Command::new(&exe_path)
        .current_dir(&install_dir)
        .spawn()
        .map_err(|e| format!("O Windows não abriu o jogo: {e}"))?;

    app.state::<AppState>().running.lock().unwrap().insert(id.clone());
    let _ = app.emit("session", serde_json::json!({ "id": id, "state": "starting" }));

    std::thread::spawn(move || {
        let prefix = folder_prefix(&install_dir);
        let mut sys = System::new();
        let launched = Instant::now();
        let mut started: Option<Instant> = None;
        loop {
            std::thread::sleep(POLL);
            let running = port_running(&mut sys, &prefix);
            match (started, running) {
                (None, true) => {
                    started = Some(Instant::now());
                    let _ = app.emit("session", serde_json::json!({ "id": id, "state": "running" }));
                }
                (None, false) if launched.elapsed() > START_TIMEOUT => break,
                (Some(_), false) => break,
                _ => {}
            }
        }

        let state = app.state::<AppState>();
        state.running.lock().unwrap().remove(&id);
        let seconds = started.map(|s| s.elapsed().as_secs()).unwrap_or(0);
        if seconds > 0 {
            let mut lib = state.library.lock().unwrap();
            if let Some(entry) = lib.games.get_mut(&id) {
                entry.total_seconds += seconds;
                entry.last_played = Some(now());
            }
            let _ = lib.save(&state.library_path);
        }
        let _ = app.emit("session", serde_json::json!({ "id": id, "state": "ended", "seconds": seconds }));
    });
    Ok(())
}

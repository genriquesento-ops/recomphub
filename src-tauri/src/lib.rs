mod catalog;
mod files;
mod github;
mod install;
mod library;
mod play;

use base64::Engine;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use catalog::GameDef;
use files::FilesStatus;
use github::Release;
use library::Library;

pub struct AppState {
    pub library: Mutex<Library>,
    pub library_path: PathBuf,
    pub latest: Mutex<HashMap<String, Release>>,
    pub running: Mutex<HashSet<String>>,
    pub installing: Mutex<HashSet<String>>,
}

/// Everything the interface shows about one game.
#[derive(Serialize)]
struct GameView {
    id: &'static str,
    title: &'static str,
    family: &'static str,
    port_name: &'static str,
    port_authors: &'static str,
    project_url: &'static str,
    needs: &'static str,
    files_help: &'static str,
    files_kind: &'static str,
    requirements: &'static str,
    /// "not_installed", "needs_files", "ready", "running" or "installing"
    status: &'static str,
    in_library: bool,
    install_dir: Option<String>,
    version: Option<String>,
    latest_version: Option<String>,
    update_available: bool,
    files: Option<FilesStatus>,
    total_seconds: u64,
    last_played: Option<u64>,
    cover: Option<String>,
}

fn data_url(path: &Path) -> Option<String> {
    let mime = match path.extension()?.to_str()?.to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        _ => return None,
    };
    let bytes = std::fs::read(path).ok()?;
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// Modification time of the port's program, for copies added from a folder (no known tag).
fn modified(path: &Path) -> Option<u64> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

fn view(state: &AppState, game: &'static GameDef) -> GameView {
    let entry = state.library.lock().unwrap().games.get(game.id).cloned();
    let latest = state.latest.lock().unwrap().get(game.id).cloned();
    let running = state.running.lock().unwrap().contains(game.id);
    let installing = state.installing.lock().unwrap().contains(game.id);

    let dir = entry.as_ref().map(|e| PathBuf::from(&e.install_dir));
    let installed = dir.as_deref().map(|d| game.matches(d)).unwrap_or(false);
    let files = dir.as_deref().filter(|_| installed).map(|d| files::status(game, d));

    let update_available = match (&entry, &latest, installed) {
        (Some(e), Some(l), true) => match &e.version {
            Some(v) => v != &l.tag,
            // Added from a folder: newer if the release came out a day after the copy's program.
            None => modified(&dir.as_ref().unwrap().join(game.exe))
                .map(|m| l.published > m + 86_400)
                .unwrap_or(false),
        },
        _ => false,
    };

    let status = if installing {
        "installing"
    } else if running {
        "running"
    } else if !installed {
        "not_installed"
    } else if files.as_ref().map(|f| f.ok).unwrap_or(false) {
        "ready"
    } else {
        "needs_files"
    };

    GameView {
        id: game.id,
        title: game.title,
        family: game.family,
        port_name: game.port_name,
        port_authors: game.port_authors,
        project_url: game.project_url,
        needs: game.needs,
        files_help: game.files_help,
        files_kind: game.files_kind,
        requirements: game.requirements,
        status,
        in_library: entry.is_some(),
        install_dir: entry.as_ref().map(|e| e.install_dir.clone()),
        version: entry.as_ref().and_then(|e| e.version.clone()),
        latest_version: latest.map(|l| l.tag),
        update_available,
        files,
        total_seconds: entry.as_ref().map(|e| e.total_seconds).unwrap_or(0),
        last_played: entry.as_ref().and_then(|e| e.last_played),
        cover: files::cover(game, dir.as_deref().filter(|_| installed)).and_then(|p| data_url(&p)),
    }
}

fn game_or_err(id: &str) -> Result<&'static GameDef, String> {
    catalog::get(id).ok_or_else(|| format!("Jogo desconhecido: {id}"))
}

fn save(state: &AppState) -> Result<(), String> {
    state.library.lock().unwrap().save(&state.library_path)
}

#[tauri::command]
fn get_library(app: AppHandle) -> Vec<GameView> {
    let state = app.state::<AppState>();
    catalog::GAMES.iter().map(|g| view(&state, g)).collect()
}

#[tauri::command]
fn get_game(app: AppHandle, id: String) -> Result<GameView, String> {
    Ok(view(&app.state::<AppState>(), game_or_err(&id)?))
}

/// Asks each project's GitHub page for its newest release. Offline is not an error here.
#[tauri::command]
async fn check_updates(app: AppHandle) -> Vec<GameView> {
    for game in catalog::GAMES {
        if let Ok(release) = github::latest(game).await {
            app.state::<AppState>().latest.lock().unwrap().insert(game.id.to_string(), release);
        }
    }
    get_library(app)
}

// Dialogs block until closed: only call these from async commands, never the main thread.
fn pick_folder(app: &AppHandle, title: &str) -> Option<PathBuf> {
    app.dialog().file().set_title(title).blocking_pick_folder().and_then(|p| p.into_path().ok())
}

fn pick_file(app: &AppHandle, title: &str) -> Option<PathBuf> {
    app.dialog()
        .file()
        .set_title(title)
        .add_filter("ROM de N64", &["z64", "n64", "v64", "rom", "bin"])
        .add_filter("Todos os arquivos", &["*"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok())
}

/// "Adicionar jogo": the player points at a folder with a port already installed.
/// Ok(None) when the player closed the dialog.
#[tauri::command]
async fn add_existing(app: AppHandle) -> Result<Option<GameView>, String> {
    let Some(dir) = pick_folder(&app, "Escolha a pasta do jogo já instalado") else {
        return Ok(None);
    };
    let Some((game, port_dir)) = catalog::identify(&dir) else {
        let names: Vec<_> = catalog::GAMES.iter().map(|g| format!("{} ({})", g.title, g.port_name)).collect();
        return Err(format!(
            "Não reconheci nenhum port nesta pasta. Os jogos suportados são: {}. Escolha a pasta onde está o programa do port (por exemplo, \"{}\").",
            names.join(", "),
            catalog::GAMES[0].exe
        ));
    };
    let state = app.state::<AppState>();
    state.library.lock().unwrap().add(game.id, &port_dir, None);
    save(&state)?;
    Ok(Some(view(&state, game)))
}

/// Installs or updates a port from its official release.
#[tauri::command]
async fn install_game(app: AppHandle, id: String) -> Result<GameView, String> {
    let game = game_or_err(&id)?;
    {
        let state = app.state::<AppState>();
        if state.running.lock().unwrap().contains(game.id) {
            return Err("Feche o jogo antes de atualizar.".into());
        }
        if !state.installing.lock().unwrap().insert(game.id.to_string()) {
            return Err("A instalação já está em andamento.".into());
        }
    }
    let result = install_inner(&app, game).await;
    let state = app.state::<AppState>();
    state.installing.lock().unwrap().remove(game.id);
    result?;
    Ok(view(&state, game))
}

async fn install_inner(app: &AppHandle, game: &'static GameDef) -> Result<(), String> {
    let release = github::latest(game).await?;
    app.state::<AppState>().latest.lock().unwrap().insert(game.id.to_string(), release.clone());

    let existing = app.state::<AppState>().library.lock().unwrap().install_dir(game.id);
    let target = match existing {
        Some(d) if game.matches(&d) => d,
        _ => app.path().app_local_data_dir().map_err(|e| e.to_string())?.join("games").join(game.id),
    };
    install::install(app, game, &release, &target).await?;

    let state = app.state::<AppState>();
    state.library.lock().unwrap().add(game.id, &target, Some(release.tag));
    save(&state)
}

/// The player picks their game files; they are checked and handed to the port.
/// Ok(None) when the player closed the dialog.
#[tauri::command]
async fn provide_files(app: AppHandle, id: String) -> Result<Option<FilesStatus>, String> {
    let game = game_or_err(&id)?;
    let dir = app
        .state::<AppState>()
        .library
        .lock()
        .unwrap()
        .install_dir(game.id)
        .ok_or("Instale o jogo primeiro.")?;
    let picked = if game.files_kind == "folder" {
        pick_folder(&app, "Escolha a pasta com o dump do jogo")
    } else {
        pick_file(&app, "Escolha o arquivo da ROM")
    };
    let Some(path) = picked else { return Ok(None) };
    let status = tauri::async_runtime::spawn_blocking(move || files::provide(game, &dir, &path))
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(status))
}

#[tauri::command]
fn play(app: AppHandle, id: String) -> Result<(), String> {
    let game = game_or_err(&id)?;
    let v = view(&app.state::<AppState>(), game);
    match v.status {
        "ready" => {}
        "running" => return Err("O jogo já está aberto.".into()),
        "needs_files" => return Err("Faltam os arquivos do jogo. Veja os detalhes na página do jogo.".into()),
        _ => return Err("O jogo não está instalado.".into()),
    }
    let dir = PathBuf::from(v.install_dir.unwrap());
    play::start(app, id, dir, game.exe)
}

#[tauri::command]
fn remove_game(app: AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.library.lock().unwrap().remove(&id);
    save(&state)
}

#[tauri::command]
fn open_project(app: AppHandle, id: String) -> Result<(), String> {
    let game = game_or_err(&id)?;
    app.opener().open_url(game.project_url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_folder(app: AppHandle, id: String) -> Result<(), String> {
    let dir = app.state::<AppState>().library.lock().unwrap().install_dir(&id).ok_or("Jogo fora da biblioteca.")?;
    app.opener().open_path(dir.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let library_path = dir.join("library.json");
            app.manage(AppState {
                library: Mutex::new(Library::load(&library_path)),
                library_path,
                latest: Mutex::new(HashMap::new()),
                running: Mutex::new(HashSet::new()),
                installing: Mutex::new(HashSet::new()),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_library,
            get_game,
            check_updates,
            add_existing,
            install_game,
            provide_files,
            play,
            remove_game,
            open_project,
            open_folder
        ])
        .run(tauri::generate_context!())
        .expect("error while running RecompHUB");
}

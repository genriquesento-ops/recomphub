mod catalog;
mod covers;
mod files;
mod github;
mod install;
mod library;
mod play;
mod shortcut;

use base64::Engine;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
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
    pub sessions: Mutex<play::Sessions>,
    pub installing: Mutex<HashSet<String>>,
    /// Game page to show, asked for by a shortcut before the page loaded.
    pub pending_open: Mutex<Option<String>>,
    pub settings: Mutex<covers::Settings>,
    pub settings_path: PathBuf,
    pub covers: covers::Dirs,
}

/// Everything the interface shows about one game.
#[derive(Serialize)]
struct GameView {
    id: String,
    /// "port" (from the catalog) or "pc_game" (added by the player)
    kind: &'static str,
    title: String,
    family: &'static str,
    port_name: &'static str,
    port_authors: &'static str,
    project_url: &'static str,
    needs: &'static str,
    files_help: &'static str,
    files_kind: &'static str,
    requirements: &'static str,
    guide: &'static [&'static str],
    guide_links: &'static [catalog::GuideLink],
    tags: Vec<String>,
    /// "not_installed", "needs_files", "ready", "starting", "running" or "installing"
    status: &'static str,
    /// Unix time the running session started.
    session_started: Option<u64>,
    in_library: bool,
    install_dir: Option<String>,
    version: Option<String>,
    latest_version: Option<String>,
    update_available: bool,
    files: Option<FilesStatus>,
    total_seconds: u64,
    last_played: Option<u64>,
    cover: Option<String>,
    /// Where the cover came from: "custom", "own_files", "downloaded" or none.
    cover_source: Option<&'static str>,
    /// A PC game's program icon, shown when there is no cover.
    icon: Option<String>,
}

/// The cover shown for a game, in order: the player's choice, their own files, a download.
fn pick_cover(state: &AppState, id: &str, own: Option<PathBuf>) -> (Option<String>, Option<&'static str>) {
    let found = covers::find(&state.covers.custom, id)
        .map(|p| (p, "custom"))
        .or_else(|| own.map(|p| (p, "own_files")))
        .or_else(|| covers::find(&state.covers.downloaded, id).map(|p| (p, "downloaded")));
    match found {
        Some((p, source)) => (data_url(&p), Some(source)),
        None => (None, None),
    }
}

fn data_url(path: &Path) -> Option<String> {
    let mime = match path.extension()?.to_str()?.to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
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
    let (running, launching, session_started) = {
        let s = state.sessions.lock().unwrap();
        (s.is_running(game.id), s.is_launching(game.id), s.started(game.id))
    };
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

    let (cover, cover_source) = pick_cover(state, game.id, files::cover(game, dir.as_deref().filter(|_| installed)));

    let status = if installing {
        "installing"
    } else if running {
        "running"
    } else if launching {
        "starting"
    } else if !installed {
        "not_installed"
    } else if files.as_ref().map(|f| f.ok).unwrap_or(false) {
        "ready"
    } else {
        "needs_files"
    };

    GameView {
        id: game.id.to_string(),
        kind: "port",
        title: game.title.to_string(),
        family: game.family,
        port_name: game.port_name,
        port_authors: game.port_authors,
        project_url: game.project_url,
        needs: game.needs,
        files_help: game.files_help,
        files_kind: game.files_kind,
        requirements: game.requirements,
        guide: game.guide,
        guide_links: game.guide_links,
        tags: entry.as_ref().map(|e| e.tags.clone()).unwrap_or_default(),
        status,
        session_started,
        in_library: entry.is_some(),
        install_dir: entry.as_ref().map(|e| e.install_dir.clone()),
        version: entry.as_ref().and_then(|e| e.version.clone()),
        latest_version: latest.map(|l| l.tag),
        update_available,
        files,
        total_seconds: entry.as_ref().map(|e| e.total_seconds).unwrap_or(0),
        last_played: entry.as_ref().and_then(|e| e.last_played),
        cover,
        cover_source,
        icon: None,
    }
}

/// A PC game the player added: RecompHUB only opens it and times it.
fn view_pc(state: &AppState, id: &str, entry: &library::Entry) -> GameView {
    let (running, launching, session_started) = {
        let s = state.sessions.lock().unwrap();
        (s.is_running(id), s.is_launching(id), s.started(id))
    };
    let exe = pc_exe(entry);
    let (cover, cover_source) = pick_cover(state, id, None);
    let status = if running {
        "running"
    } else if launching {
        "starting"
    } else if exe.as_deref().map(Path::is_file).unwrap_or(false) {
        "ready"
    } else {
        "missing"
    };
    GameView {
        id: id.to_string(),
        kind: "pc_game",
        title: entry.title.clone().unwrap_or_else(|| "Jogo sem nome".into()),
        family: "PC",
        port_name: "",
        port_authors: "",
        project_url: "",
        needs: "",
        files_help: "",
        files_kind: "",
        requirements: "",
        guide: &[],
        guide_links: &[],
        tags: entry.tags.clone(),
        status,
        session_started,
        in_library: true,
        install_dir: exe.map(|p| p.display().to_string()),
        version: None,
        latest_version: None,
        update_available: false,
        files: None,
        total_seconds: entry.total_seconds,
        last_played: entry.last_played,
        cover,
        cover_source,
        icon: covers::find(&state.covers.icons, id).and_then(|p| data_url(&p)),
    }
}

fn pc_exe(entry: &library::Entry) -> Option<PathBuf> {
    entry.exe.as_ref().map(|e| Path::new(&entry.install_dir).join(e))
}

/// Any game the interface can ask about: a catalog port or a PC game in the library.
fn view_any(state: &AppState, id: &str) -> Result<GameView, String> {
    if let Some(game) = catalog::get(id) {
        return Ok(view(state, game));
    }
    let entry = state.library.lock().unwrap().games.get(id).cloned();
    match entry {
        Some(e) if e.kind == library::EntryKind::PcGame => Ok(view_pc(state, id, &e)),
        _ => Err("Este jogo não está mais na biblioteca.".into()),
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
    let mut list: Vec<GameView> = catalog::GAMES.iter().map(|g| view(&state, g)).collect();
    let pc: Vec<(String, library::Entry)> = state
        .library
        .lock()
        .unwrap()
        .games
        .iter()
        .filter(|(_, e)| e.kind == library::EntryKind::PcGame)
        .map(|(id, e)| (id.clone(), e.clone()))
        .collect();
    let mut pc: Vec<GameView> = pc.iter().map(|(id, e)| view_pc(&state, id, e)).collect();
    pc.sort_by_key(|g| g.title.to_lowercase());
    list.extend(pc);
    list
}

#[tauri::command]
fn get_game(app: AppHandle, id: String) -> Result<GameView, String> {
    view_any(&app.state::<AppState>(), &id)
}

/// Folders a PC game must not be timed by: everything else running there would count.
fn too_broad(dir: &Path) -> bool {
    if dir.parent().is_none() {
        return true;
    }
    let norm = |p: &Path| p.to_string_lossy().trim_end_matches('\\').to_lowercase();
    let here = norm(dir);
    ["WINDIR", "ProgramFiles", "ProgramFiles(x86)", "USERPROFILE"]
        .iter()
        .filter_map(|v| std::env::var_os(v))
        .map(PathBuf::from)
        .flat_map(|p| [p.join("Desktop"), p.join("Downloads"), p])
        .any(|p| norm(&p) == here)
}

/// "Adicionar outro jogo de PC": the player points at the game's program.
/// Ok(None) when the player closed the dialog.
#[tauri::command]
async fn add_pc_game(app: AppHandle) -> Result<Option<GameView>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Escolha o programa do jogo (.exe)")
        .add_filter("Programa", &["exe"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok());
    let Some(exe) = picked else { return Ok(None) };
    let dir = exe.parent().map(Path::to_path_buf).ok_or("Escolha um arquivo .exe.")?;

    // A catalog port gets its full page (files check, updates) instead.
    if let Some((game, port_dir)) = catalog::identify(&dir) {
        let state = app.state::<AppState>();
        state.library.lock().unwrap().add(game.id, &port_dir, None);
        save(&state)?;
        return Ok(Some(view(&state, game)));
    }
    if too_broad(&dir) {
        return Err(format!(
            "O jogo está direto em {}. Coloque-o numa pasta só dele: o RecompHUB conta o tempo de jogo por pasta, e aqui contaria outros programas também.",
            dir.display()
        ));
    }

    let key = exe.to_string_lossy().to_lowercase();
    let id = format!("pc-{:016x}", xxhash_rust::xxh3::xxh3_64(key.as_bytes()));
    let state = app.state::<AppState>();
    if state.library.lock().unwrap().games.contains_key(&id) {
        return Err("Este jogo já está na biblioteca.".into());
    }
    let title = exe.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Jogo".into());
    state.library.lock().unwrap().add_pc_game(&id, &exe, &title);
    save(&state)?;
    // Without a cover the card shows the program's own icon; failing to read it is harmless.
    let _ = covers::extract_icon(&state.covers, &id, &exe);
    view_any(&state, &id).map(Some)
}

#[tauri::command]
fn rename_game(app: AppHandle, id: String, title: String) -> Result<GameView, String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("O nome não pode ficar vazio.".into());
    }
    let state = app.state::<AppState>();
    {
        let mut lib = state.library.lock().unwrap();
        let entry = lib.games.get_mut(&id).filter(|e| e.kind == library::EntryKind::PcGame);
        entry.ok_or("Só jogos adicionados por você podem ser renomeados.")?.title = Some(title);
    }
    save(&state)?;
    view_any(&state, &id)
}

/// The player's labels: trimmed, without repeats, at most 12.
#[tauri::command]
fn set_tags(app: AppHandle, id: String, tags: Vec<String>) -> Result<GameView, String> {
    let mut clean: Vec<String> = Vec::new();
    for t in tags {
        let t = t.trim().chars().take(30).collect::<String>();
        if !t.is_empty() && !clean.iter().any(|c| c.to_lowercase() == t.to_lowercase()) {
            clean.push(t);
        }
    }
    clean.truncate(12);
    let state = app.state::<AppState>();
    {
        let mut lib = state.library.lock().unwrap();
        lib.games.get_mut(&id).ok_or("Adicione o jogo à biblioteca primeiro.")?.tags = clean;
    }
    save(&state)?;
    view_any(&state, &id)
}

/// "Escolher capa": any game in the library can use a picture from the player's PC.
#[tauri::command]
async fn choose_cover(app: AppHandle, id: String) -> Result<Option<GameView>, String> {
    let state = app.state::<AppState>();
    if !state.library.lock().unwrap().games.contains_key(&id) {
        return Err("Adicione o jogo à biblioteca primeiro.".into());
    }
    let picked = app
        .dialog()
        .file()
        .set_title("Escolha a imagem da capa")
        .add_filter("Imagem", covers::IMAGE_EXTS)
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok());
    let Some(picture) = picked else { return Ok(None) };
    covers::set_custom(&state.covers, &id, &picture)?;
    view_any(&state, &id).map(Some)
}

#[tauri::command]
fn clear_cover(app: AppHandle, id: String) -> Result<GameView, String> {
    let state = app.state::<AppState>();
    covers::clear(&state.covers, &id);
    view_any(&state, &id)
}

#[derive(Serialize)]
struct CoverDownload {
    downloaded: usize,
    missing: Vec<String>,
    library: Vec<GameView>,
}

/// "Baixar capas que faltam", as PCSX2's cover downloader: for each game in the library
/// without a cover, the catalog's curated addresses first, then the player's templates.
#[tauri::command]
async fn download_covers(app: AppHandle) -> Result<CoverDownload, String> {
    let todo: Vec<(String, String, Vec<String>)> = {
        let state = app.state::<AppState>();
        let templates = state.settings.lock().unwrap().cover_templates.clone();
        get_library(app.clone())
            .into_iter()
            .filter(|g| g.in_library && g.cover.is_none())
            .map(|g| {
                let game = catalog::get(&g.id);
                let serial = game.and_then(files::serial);
                let mut urls: Vec<String> = game.map(|d| d.cover_urls.iter().map(|u| u.to_string()).collect()).unwrap_or_default();
                urls.extend(templates.iter().filter_map(|t| covers::fill(t, serial.as_deref(), &g.title)));
                (g.id, g.title, urls)
            })
            .collect()
    };
    let mut downloaded = 0;
    let mut missing = Vec::new();
    let state = app.state::<AppState>();
    for (id, title, urls) in todo {
        if !urls.is_empty() && covers::download(&state.covers, &id, &urls).await {
            downloaded += 1;
        } else {
            missing.push(title);
        }
    }
    Ok(CoverDownload { downloaded, missing, library: get_library(app.clone()) })
}

#[tauri::command]
fn get_cover_templates(app: AppHandle) -> Vec<String> {
    app.state::<AppState>().settings.lock().unwrap().cover_templates.clone()
}

#[tauri::command]
fn set_cover_templates(app: AppHandle, lines: Vec<String>) -> Result<Vec<String>, String> {
    let clean = covers::clean_templates(&lines)?;
    let state = app.state::<AppState>();
    let mut settings = state.settings.lock().unwrap();
    settings.cover_templates = clean.clone();
    settings.save(&state.settings_path)?;
    Ok(clean)
}

#[tauri::command]
fn open_link(app: AppHandle, url: String) -> Result<(), String> {
    // Only pages the catalog lists: never an address from elsewhere.
    let known = catalog::GAMES
        .iter()
        .any(|g| g.project_url == url || g.guide_links.iter().any(|l| l.url == url));
    if !known {
        return Err("Link fora do catálogo.".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
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
        if state.sessions.lock().unwrap().is_running(game.id) {
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
    let state = app.state::<AppState>();
    let v = view_any(&state, &id)?;
    match v.status {
        "ready" => {}
        "running" | "starting" => return Err("O jogo já está aberto.".into()),
        "needs_files" => return Err("Faltam os arquivos do jogo. Veja os detalhes na página do jogo.".into()),
        "installing" => return Err("Espere a instalação terminar.".into()),
        "missing" => return Err("O programa do jogo não está mais no lugar. Ele foi movido ou apagado?".into()),
        _ => return Err("O jogo não está instalado.".into()),
    }
    let entry = state.library.lock().unwrap().games.get(&id).cloned().ok_or("Jogo fora da biblioteca.")?;
    let dir = PathBuf::from(&entry.install_dir);
    let exe = match catalog::get(&id) {
        Some(game) => game.exe.to_string(),
        None => entry.exe.clone().ok_or("Jogo sem programa definido.")?,
    };
    play::start(&app, &id, &dir, &exe)
}

#[tauri::command]
fn stop_game(app: AppHandle, id: String) -> Result<(), String> {
    let dir = app.state::<AppState>().library.lock().unwrap().install_dir(&id).ok_or("Jogo fora da biblioteca.")?;
    play::stop(&dir).map(|_| ())
}

#[tauri::command]
fn open_saves(app: AppHandle, id: String) -> Result<(), String> {
    let game = game_or_err(&id)?;
    let dir = app.state::<AppState>().library.lock().unwrap().install_dir(&id).ok_or("Jogo fora da biblioteca.")?;
    let saves = files::save_dir(game, &dir).ok_or("O jogo ainda não criou a pasta de saves. Jogue uma vez e tente de novo.")?;
    app.opener().open_path(saves.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

/// `location`: "desktop" or "start_menu".
#[tauri::command]
fn create_shortcut(app: AppHandle, id: String, location: String) -> Result<String, String> {
    let entry = app.state::<AppState>().library.lock().unwrap().games.get(&id).cloned().ok_or("Jogo fora da biblioteca.")?;
    let dir = PathBuf::from(&entry.install_dir);
    let folder = match location.as_str() {
        "desktop" => app.path().desktop_dir().map_err(|e| e.to_string())?,
        _ => app
            .path()
            .data_dir()
            .map_err(|e| e.to_string())?
            .join("Microsoft\\Windows\\Start Menu\\Programs\\RecompHUB"),
    };
    let (title, icon) = match catalog::get(&id) {
        Some(game) => {
            let icon = Some(dir.join(game.icon)).filter(|p| p.is_file()).unwrap_or_else(|| dir.join(game.exe));
            (game.title.to_string(), icon)
        }
        // A PC game's program carries its own icon.
        None => (
            entry.title.clone().unwrap_or_else(|| "Jogo".into()),
            pc_exe(&entry).ok_or("Jogo sem programa definido.")?,
        ),
    };
    shortcut::create(&folder, &title, &id, &icon).map(|p| p.display().to_string())
}

#[tauri::command]
fn take_pending_open(app: AppHandle) -> Option<String> {
    app.state::<AppState>().pending_open.lock().unwrap().take()
}

/// "--play <id>" (from a shortcut): open that game if it is ready, else show its page.
fn handle_args(app: &AppHandle, args: &[String]) {
    let Some(i) = args.iter().position(|a| a == "--play") else { return };
    let Some(id) = args.get(i + 1) else { return };
    let Ok(v) = view_any(&app.state::<AppState>(), id) else { return };
    let ready = v.status == "ready";
    let launched = ready && play(app.clone(), id.clone()).is_ok();
    if !launched {
        // The page may not be loaded yet (RecompHUB just started): it asks on load.
        *app.state::<AppState>().pending_open.lock().unwrap() = Some(id.clone());
        let _ = app.emit("open-game", id);
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
    }
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
        // A shortcut opened while RecompHUB is running goes to the open window.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
            handle_args(app, &args);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let library_path = dir.join("library.json");
            let settings_path = dir.join("settings.json");
            app.manage(AppState {
                settings: Mutex::new(covers::Settings::load(&settings_path)),
                settings_path,
                covers: covers::Dirs::new(&dir),
                library: Mutex::new(Library::load(&library_path)),
                library_path,
                latest: Mutex::new(HashMap::new()),
                sessions: Mutex::new(play::Sessions::default()),
                installing: Mutex::new(HashSet::new()),
                pending_open: Mutex::new(None),
            });
            play::spawn_watcher(app.handle().clone());
            let args: Vec<String> = std::env::args().collect();
            handle_args(app.handle(), &args);
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
            stop_game,
            open_saves,
            create_shortcut,
            add_pc_game,
            rename_game,
            set_tags,
            open_link,
            choose_cover,
            clear_cover,
            download_covers,
            get_cover_templates,
            set_cover_templates,
            take_pending_open,
            remove_game,
            open_project,
            open_folder
        ])
        .build(tauri::generate_context!())
        .expect("error while starting RecompHUB")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                play::flush(app);
            }
        });
}

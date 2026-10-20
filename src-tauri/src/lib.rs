mod autosave;
mod catalog;
mod catalog_sync;
mod community;
mod covers;
mod files;
mod github;
mod history;
mod install;
mod library;
mod ludusavi;
mod meta;
mod play;
mod profiles;
mod saves;
mod shortcut;
mod tray;

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
    /// Where RecompHUB installs ports; only folders under it are ever deleted.
    pub games_root: PathBuf,
    /// Saves moved out of uninstalled ports, one folder per port id.
    pub pending_root: PathBuf,
    /// Dated .zip files from "Backup dos saves".
    pub backups_root: PathBuf,
    /// Automatic backups, one folder per game (autosave.rs).
    pub auto_root: PathBuf,
    /// Aba Comunidade cache (community.json).
    pub community_path: PathBuf,
    /// SteamGridDB and IGDB lookups (meta/<id>.json and pictures).
    pub meta_dir: PathBuf,
    /// One record per play session (history.json).
    pub history: Mutex<history::History>,
    pub history_path: PathBuf,
    /// Players on this PC (profiles.rs); progress of the inactive ones in profiles_dir.
    pub profiles: Mutex<profiles::Profiles>,
    pub profiles_path: PathBuf,
    pub profiles_dir: PathBuf,
}

/// Everything the interface shows about one game.
#[derive(Serialize)]
struct GameView {
    id: String,
    /// "port" (from the catalog) or "pc_game" (added by the player)
    kind: &'static str,
    title: String,
    family: &'static str,
    port_name: String,
    port_authors: String,
    project_url: String,
    /// Community port: its GitHub repository.
    repo: Option<String>,
    needs: &'static str,
    files_help: &'static str,
    files_kind: &'static str,
    requirements: &'static str,
    guide: &'static [String],
    guide_links: &'static [catalog::GuideLink],
    tags: Vec<String>,
    /// The catalog's own tags (platform, genre), for the Descobrir tab.
    catalog_tags: &'static [String],
    /// "not_installed", "needs_files", "ready", "starting", "running" or "installing"
    status: &'static str,
    /// Unix time the running session started.
    session_started: Option<u64>,
    in_library: bool,
    install_dir: Option<String>,
    version: Option<String>,
    latest_version: Option<String>,
    update_available: bool,
    /// The player turned off "versão nova" notices for this port.
    ignore_updates: bool,
    /// Installed by RecompHUB (it may uninstall it), not added from the player's folder.
    installed_by_hub: bool,
    /// Saves kept from an uninstall, waiting for the next install.
    pending_saves: bool,
    favorite: bool,
    hidden: bool,
    launch_args: String,
    env: Vec<library::EnvVar>,
    /// library::Completion as text ("none", "playing", "beaten"…).
    completion: library::Completion,
    play_count: u32,
    notes: String,
    files: Option<FilesStatus>,
    total_seconds: u64,
    last_played: Option<u64>,
    cover: Option<String>,
    /// Where the cover came from: "custom", "own_files", "downloaded", "api" or none.
    cover_source: Option<&'static str>,
    /// A PC game's program icon, shown when there is no cover.
    icon: Option<String>,
}

/// The cover shown for a game, in order: the player's choice, their own files, a download.
fn pick_cover(state: &AppState, id: &str, own: Option<PathBuf>) -> (Option<String>, Option<&'static str>) {
    let found = covers::find(&state.covers.custom, id)
        .map(|p| (p, "custom"))
        .or_else(|| own.map(|p| (p, "own_files")))
        .or_else(|| covers::find(&state.covers.downloaded, id).map(|p| (p, "downloaded")))
        .or_else(|| meta::image(&state.meta_dir, id, "cover").map(|p| (p, "api")));
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
    let entry = state.library.lock().unwrap().games.get(game.id.as_str()).cloned();
    let latest = state.latest.lock().unwrap().get(game.id.as_str()).cloned();
    let (running, launching, session_started) = {
        let s = state.sessions.lock().unwrap();
        (s.is_running(game.id.as_str()), s.is_launching(game.id.as_str()), s.started(game.id.as_str()))
    };
    let installing = state.installing.lock().unwrap().contains(game.id.as_str());

    let dir = entry.as_ref().map(|e| PathBuf::from(&e.install_dir));
    let installed = dir.as_deref().map(|d| game.matches(d)).unwrap_or(false);
    let files = dir.as_deref().filter(|_| installed).map(|d| files::status(game, d));

    let update_available = match (&entry, &latest, installed) {
        (Some(e), Some(l), true) if !e.ignore_updates => match &e.version {
            Some(v) => v != &l.tag,
            // Added from a folder: newer if the release came out a day after the copy's program.
            None => modified(&dir.as_ref().unwrap().join(&game.exe))
                .map(|m| l.published > m + 86_400)
                .unwrap_or(false),
        },
        _ => false,
    };

    let (cover, cover_source) = pick_cover(state, &game.id, files::cover(game, dir.as_deref().filter(|_| installed)));

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
        family: game.family.as_str(),
        port_name: game.port_name.clone(),
        port_authors: game.port_authors.clone(),
        project_url: game.project_url.clone(),
        repo: None,
        needs: game.needs.as_str(),
        files_help: game.files_help.as_str(),
        files_kind: game.files_kind(),
        requirements: game.requirements.as_str(),
        guide: &game.guide,
        guide_links: &game.guide_links,
        catalog_tags: &game.tags,
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
        ignore_updates: entry.as_ref().map(|e| e.ignore_updates).unwrap_or(false),
        installed_by_hub: entry.as_ref().map(|e| e.version.is_some() && Path::new(&e.install_dir).starts_with(&state.games_root)).unwrap_or(false),
        pending_saves: state.pending_root.join(&game.id).is_dir(),
        favorite: entry.as_ref().map(|e| e.favorite).unwrap_or(false),
        hidden: entry.as_ref().map(|e| e.hidden).unwrap_or(false),
        launch_args: entry.as_ref().map(|e| e.launch_args.clone()).unwrap_or_default(),
        env: entry.as_ref().map(|e| e.env.clone()).unwrap_or_default(),
        completion: entry.as_ref().map(|e| e.completion).unwrap_or_default(),
        play_count: entry.as_ref().map(|e| e.play_count).unwrap_or(0),
        notes: entry.as_ref().map(|e| e.notes.clone()).unwrap_or_default(),
    }
}

/// A PC game the player added: RecompHUB only opens it and times it.
fn view_pc(state: &AppState, id: &str, entry: &library::Entry) -> GameView {
    let (running, launching, session_started) = {
        let s = state.sessions.lock().unwrap();
        (s.is_running(id), s.is_launching(id), s.started(id))
    };
    let community = entry.kind == library::EntryKind::Community;
    let installing = state.installing.lock().unwrap().contains(id);
    let exe = pc_exe(entry);
    let (cover, cover_source) = pick_cover(state, id, None);
    let status = if installing {
        "installing"
    } else if running {
        "running"
    } else if launching {
        "starting"
    } else if exe.as_deref().map(Path::is_file).unwrap_or(false) {
        "ready"
    } else if community && entry.exe.is_none() {
        "needs_exe"
    } else {
        "missing"
    };
    let repo = entry.repo.clone().filter(|_| community);
    let latest = repo.as_ref().and_then(|_| state.latest.lock().unwrap().get(id).map(|r| r.tag.clone()));
    let installed_by_hub = community && Path::new(&entry.install_dir).starts_with(&state.games_root);
    GameView {
        id: id.to_string(),
        kind: if community { "community" } else { "pc_game" },
        title: entry.title.clone().unwrap_or_else(|| "Jogo sem nome".into()),
        family: if community { "Comunidade" } else { "PC" },
        port_name: repo.as_ref().and_then(|r| r.split('/').nth(1)).unwrap_or("").to_string(),
        port_authors: repo.as_ref().and_then(|r| r.split('/').next()).unwrap_or("").to_string(),
        project_url: repo.as_ref().map(|r| format!("https://github.com/{r}")).unwrap_or_default(),
        repo: repo.clone(),
        needs: "",
        files_help: "",
        files_kind: "",
        requirements: "",
        guide: &[],
        guide_links: &[],
        catalog_tags: &[],
        tags: entry.tags.clone(),
        status,
        session_started,
        in_library: true,
        install_dir: if community { Some(entry.install_dir.clone()) } else { exe.map(|p| p.display().to_string()) },
        version: entry.version.clone().filter(|_| community),
        update_available: !entry.ignore_updates
            && latest.is_some()
            && latest != entry.version
            && !matches!(status, "installing" | "running" | "starting"),
        latest_version: latest,
        files: None,
        total_seconds: entry.total_seconds,
        last_played: entry.last_played,
        cover,
        cover_source,
        icon: covers::find(&state.covers.icons, id).and_then(|p| data_url(&p)),
        ignore_updates: entry.ignore_updates,
        installed_by_hub,
        pending_saves: state.pending_root.join(id).is_dir(),
        favorite: entry.favorite,
        hidden: entry.hidden,
        launch_args: entry.launch_args.clone(),
        env: entry.env.clone(),
        completion: entry.completion,
        play_count: entry.play_count,
        notes: entry.notes.clone(),
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
        Some(e) if e.kind != library::EntryKind::Port => Ok(view_pc(state, id, &e)),
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
    let mut list: Vec<GameView> = catalog::games().iter().map(|g| view(&state, g)).collect();
    let pc: Vec<(String, library::Entry)> = state
        .library
        .lock()
        .unwrap()
        .games
        .iter()
        .filter(|(_, e)| e.kind != library::EntryKind::Port)
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
        state.library.lock().unwrap().add(&game.id, &port_dir, None);
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
        let entry = lib.games.get_mut(&id).filter(|e| e.kind != library::EntryKind::Port);
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
    // Only pages the catalog lists, the community list and the GitHub pages of the
    // community ports it shows: never an address from elsewhere.
    let state = app.state::<AppState>();
    let known = catalog::games().iter().any(|g| g.links().any(|l| l == url))
        || url == community::LIST_PAGE
        || url == meta::SGDB_KEY_PAGE
        || url == meta::IGDB_KEY_PAGE
        || url == ludusavi::PROJECT_PAGE
        || known_igdb_page(&state, &url)
        || url
            .strip_prefix("https://github.com/")
            .map(|repo| known_community_repo(&state, repo))
            .unwrap_or(false);
    if !known {
        return Err("Link fora do catálogo.".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Asks each project's GitHub page for its newest release. Offline is not an error here.
#[tauri::command]
async fn check_updates(app: AppHandle) -> Vec<GameView> {
    // A newer catalog from the project's repository first, so new ports are checked too.
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = catalog_sync::refresh(&dir).await;
    }
    for game in catalog::games() {
        if let Ok(release) = github::latest(game).await {
            app.state::<AppState>().latest.lock().unwrap().insert(game.id.to_string(), release);
        }
    }
    // Community ports in the library: one GitHub call each.
    let community_ports: Vec<(String, String)> = app
        .state::<AppState>()
        .library
        .lock()
        .unwrap()
        .games
        .iter()
        .filter(|(_, e)| e.kind == library::EntryKind::Community)
        .filter_map(|(id, e)| Some((id.clone(), e.repo.clone()?)))
        .collect();
    for (id, repo) in community_ports {
        if let Ok(Some(r)) = community::latest_release(&repo).await {
            let release = github::Release { tag: r.tag, published: r.published, asset_url: String::new(), asset_size: 0, sha256_url: None };
            app.state::<AppState>().latest.lock().unwrap().insert(id, release);
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
        let names: Vec<_> = catalog::games().iter().map(|g| format!("{} ({})", g.title, g.port_name)).collect();
        return Err(format!(
            "Não reconheci nenhum port nesta pasta. Os jogos do catálogo são: {}. Escolha a pasta onde está o programa do port.",
            names.join(", ")
        ));
    };
    let state = app.state::<AppState>();
    state.library.lock().unwrap().add(&game.id, &port_dir, None);
    save(&state)?;
    Ok(Some(view(&state, game)))
}

/// Installs or updates a port from its official release.
#[tauri::command]
async fn install_game(app: AppHandle, id: String, tag: Option<String>) -> Result<GameView, String> {
    let game = game_or_err(&id)?;
    {
        let state = app.state::<AppState>();
        if state.sessions.lock().unwrap().is_running(game.id.as_str()) {
            return Err("Feche o jogo antes de atualizar.".into());
        }
        if !state.installing.lock().unwrap().insert(game.id.to_string()) {
            return Err("A instalação já está em andamento.".into());
        }
    }
    let result = install_inner(&app, game, tag).await;
    let state = app.state::<AppState>();
    state.installing.lock().unwrap().remove(game.id.as_str());
    result?;
    Ok(view(&state, game))
}

/// Installs the newest release, or `tag` ("Escolher versão"). An older pick turns off the
/// "versão nova" notice so the port is not swapped back behind the player's back.
async fn install_inner(app: &AppHandle, game: &'static GameDef, tag: Option<String>) -> Result<(), String> {
    let list = github::releases(game, 20).await?;
    let newest = list.first().cloned().ok_or("Nenhuma versão para Windows encontrada na página do projeto.")?;
    let release = match &tag {
        Some(t) => list.iter().find(|r| &r.tag == t).cloned().ok_or(format!("A versão {t} não está mais disponível."))?,
        None => newest.clone(),
    };
    let state = app.state::<AppState>();
    state.latest.lock().unwrap().insert(game.id.to_string(), newest.clone());

    let existing = state.library.lock().unwrap().install_dir(game.id.as_str());
    let target = match existing {
        Some(d) if game.matches(&d) => d,
        _ => state.games_root.join(game.id.as_str()),
    };
    install::install(app, game, &release, &target).await?;
    // Saves kept from an earlier uninstall go back in place.
    saves::restore(&state.pending_root.join(&game.id), &target)?;

    {
        let mut lib = state.library.lock().unwrap();
        lib.add(&game.id, &target, Some(release.tag.clone()));
        if let Some(e) = lib.games.get_mut(&game.id) {
            e.ignore_updates = release.tag != newest.tag;
        }
    }
    save(&state)
}

/// Changes one library entry and saves; the closure returns false to refuse.
fn update_entry(app: &AppHandle, id: &str, f: impl FnOnce(&mut library::Entry)) -> Result<GameView, String> {
    let state = app.state::<AppState>();
    {
        let mut lib = state.library.lock().unwrap();
        f(lib.games.get_mut(id).ok_or("Adicione o jogo à biblioteca primeiro.")?);
    }
    save(&state)?;
    tray::refresh(app);
    view_any(&state, id)
}

#[tauri::command]
fn set_favorite(app: AppHandle, id: String, favorite: bool) -> Result<GameView, String> {
    update_entry(&app, &id, |e| e.favorite = favorite)
}

#[tauri::command]
fn set_hidden(app: AppHandle, id: String, hidden: bool) -> Result<GameView, String> {
    update_entry(&app, &id, |e| e.hidden = hidden)
}

#[tauri::command]
fn set_completion(app: AppHandle, id: String, completion: String) -> Result<GameView, String> {
    let c = library::Completion::parse(&completion).ok_or("Estado desconhecido.")?;
    update_entry(&app, &id, |e| e.completion = c)
}

#[tauri::command]
fn set_notes(app: AppHandle, id: String, notes: String) -> Result<GameView, String> {
    if notes.chars().count() > 10_000 {
        return Err("As anotações passam de 10 mil caracteres.".into());
    }
    update_entry(&app, &id, |e| e.notes = notes.trim_end().to_string())
}

#[derive(Serialize)]
struct GameHistory {
    /// Newest first, at most 30.
    sessions: Vec<history::Record>,
    /// Sessions in the history (older ones may have been played before RecompHUB 0.12).
    recorded: u32,
    average_seconds: u64,
    longest_seconds: u64,
}

#[tauri::command]
fn get_history(app: AppHandle, id: String) -> GameHistory {
    let state = app.state::<AppState>();
    let profile = state.profiles.lock().unwrap().active.clone();
    let h = state.history.lock().unwrap();
    let all = h.of(&id, &profile, usize::MAX);
    let total: u64 = all.iter().map(|r| r.seconds).sum();
    GameHistory {
        sessions: all.iter().take(30).cloned().collect(),
        recorded: all.len() as u32,
        average_seconds: if all.is_empty() { 0 } else { total / all.len() as u64 },
        longest_seconds: all.iter().map(|r| r.seconds).max().unwrap_or(0),
    }
}

#[derive(Serialize)]
struct StatsView {
    #[serde(flatten)]
    stats: history::Stats,
    /// id -> name, for games no longer in the library too.
    titles: HashMap<String, String>,
    /// Counted by library::Completion, for games in the library.
    completion: HashMap<String, u32>,
    library_seconds: u64,
    library_games: u32,
}

/// "Estatísticas". `offset_minutes` is the page's own time zone (minutes east of UTC), so
/// days and weeks start at the player's midnight.
#[tauri::command]
fn get_stats(app: AppHandle, offset_minutes: i64) -> StatsView {
    let stats = {
        let state = app.state::<AppState>();
        let profile = state.profiles.lock().unwrap().active.clone();
        let h = state.history.lock().unwrap();
        history::stats(&h, &profile, library::now(), offset_minutes.clamp(-14 * 60, 14 * 60) * 60, 12)
    };
    let library = get_library(app.clone());
    let mut titles: HashMap<String, String> = library.iter().map(|g| (g.id.clone(), g.title.clone())).collect();
    {
        let state = app.state::<AppState>();
        let lib = state.library.lock().unwrap();
        for (id, e) in &lib.removed {
            titles.entry(id.clone()).or_insert_with(|| e.title.clone().unwrap_or_else(|| id.clone()));
        }
    }
    let mut completion: HashMap<String, u32> = HashMap::new();
    for g in library.iter().filter(|g| g.in_library) {
        let key = match g.completion {
            library::Completion::None if g.total_seconds == 0 => library::Completion::NotPlayed,
            library::Completion::None => library::Completion::Playing,
            c => c,
        };
        let key = serde_json::to_value(key).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
        *completion.entry(key).or_default() += 1;
    }
    let in_library: Vec<&GameView> = library.iter().filter(|g| g.in_library).collect();
    StatsView {
        stats,
        titles,
        completion,
        library_seconds: in_library.iter().map(|g| g.total_seconds).sum(),
        library_games: in_library.len() as u32,
    }
}

/// "Opções de inicialização": arguments and environment variables for the game's program.
#[tauri::command]
fn set_launch_options(app: AppHandle, id: String, args: String, env: Vec<library::EnvVar>) -> Result<GameView, String> {
    let args = args.trim().chars().take(1000).collect::<String>();
    let mut clean: Vec<library::EnvVar> = Vec::new();
    for v in env {
        let name = v.name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        if !play::valid_env_name(&name) {
            return Err(format!("\"{name}\" não é um nome de variável válido: use letras, números e _, sem começar com número."));
        }
        clean.retain(|c| !c.name.eq_ignore_ascii_case(&name));
        clean.push(library::EnvVar { name, value: v.value.chars().take(2000).collect() });
    }
    clean.truncate(30);
    update_entry(&app, &id, |e| {
        e.launch_args = args;
        e.env = clean;
    })
}

#[derive(Serialize)]
struct AppSettings {
    close_to_tray: bool,
    show_hidden: bool,
    version: &'static str,
    /// Only whether keys are saved; the keys themselves never go back to the page.
    has_steamgriddb: bool,
    has_igdb: bool,
    sgdb_key_page: &'static str,
    igdb_key_page: &'static str,
}

#[tauri::command]
fn get_settings(app: AppHandle) -> AppSettings {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let keys = s.keys();
    AppSettings {
        close_to_tray: s.close_to_tray,
        show_hidden: s.show_hidden,
        version: env!("CARGO_PKG_VERSION"),
        has_steamgriddb: !keys.sgdb.is_empty(),
        has_igdb: keys.igdb(),
        sgdb_key_page: meta::SGDB_KEY_PAGE,
        igdb_key_page: meta::IGDB_KEY_PAGE,
    }
}

#[derive(Serialize)]
struct KeyTest {
    ok: bool,
    message: String,
}

/// "Salvar e testar": an empty field keeps the key already saved; `clear` forgets all of them.
#[tauri::command]
async fn set_api_keys(
    app: AppHandle,
    steamgriddb: String,
    igdb_client_id: String,
    igdb_client_secret: String,
    clear: bool,
) -> Result<Vec<KeyTest>, String> {
    let values: Vec<String> = [steamgriddb, igdb_client_id, igdb_client_secret].iter().map(|v| v.trim().to_string()).collect();
    if values.iter().any(|v| v.len() > 200 || v.chars().any(|c| c.is_whitespace() || c.is_control())) {
        return Err("Uma das chaves tem espaços ou é longa demais. Copie de novo do site.".into());
    }
    let keys = {
        let state = app.state::<AppState>();
        let mut guard = state.settings.lock().unwrap();
        let s: &mut covers::Settings = &mut guard;
        for (field, value) in [&mut s.steamgriddb_key, &mut s.igdb_client_id, &mut s.igdb_client_secret].into_iter().zip(values) {
            if clear {
                field.clear();
            }
            if !value.is_empty() {
                *field = value;
            }
        }
        s.save(&state.settings_path)?;
        s.keys()
    };
    Ok(meta::test(&keys).await.into_iter().map(|(ok, message)| KeyTest { ok, message }).collect())
}

/// What the game page shows from SteamGridDB and IGDB.
#[derive(Serialize)]
struct MetaView {
    /// The player has keys for at least one site.
    keys: bool,
    /// Looked up at least once (found or not).
    cached: bool,
    found: bool,
    term: String,
    name: Option<String>,
    summary: Option<String>,
    released: Option<i64>,
    genres: Vec<String>,
    igdb_url: Option<String>,
    cover_from: Option<String>,
    banner_from: Option<String>,
    banner: Option<String>,
    errors: Vec<String>,
    game: GameView,
}

fn meta_view(state: &AppState, id: &str, meta: Option<meta::Meta>, keys: bool) -> Result<MetaView, String> {
    let game = view_any(state, id)?;
    let m = meta.unwrap_or_default();
    Ok(MetaView {
        keys,
        cached: m.fetched_at > 0,
        found: m.found(),
        term: m.term,
        name: m.name,
        summary: m.summary,
        released: m.released,
        genres: m.genres,
        igdb_url: m.igdb_url,
        cover_from: m.cover_from,
        banner_from: m.banner_from,
        banner: meta::image(&state.meta_dir, id, "banner").and_then(|p| data_url(&p)),
        errors: m.errors,
        game,
    })
}

/// The game page asks for its information: from the cache when there is one; looked up
/// only when there is none yet (or a miss is a week old) and the player has keys.
/// `term` (a name the player typed) or `again` always looks it up.
#[tauri::command]
async fn game_meta(app: AppHandle, id: String, term: Option<String>, again: bool) -> Result<MetaView, String> {
    let (keys, dir, title) = {
        let state = app.state::<AppState>();
        if !state.library.lock().unwrap().games.contains_key(&id) {
            return Err("Adicione o jogo à biblioteca primeiro.".into());
        }
        let keys = state.settings.lock().unwrap().keys();
        (keys, state.meta_dir.clone(), view_any(&state, &id)?.title)
    };
    let cached = meta::load(&dir, &id);
    let term = term.map(|t| t.trim().to_string()).filter(|t| !t.is_empty() && t.len() <= 120);
    let wanted = term.is_some() || again || meta::stale(cached.as_ref(), library::now());
    let meta = if wanted && keys.any() {
        let term = term.or_else(|| cached.as_ref().map(|m| m.term.clone()).filter(|t| !t.is_empty())).unwrap_or(title);
        Some(meta::fetch(&dir, &id, &term, &keys).await)
    } else {
        cached
    };
    meta_view(&app.state::<AppState>(), &id, meta, keys.any())
}

#[derive(Serialize)]
struct MetaAll {
    looked_up: usize,
    found: usize,
    library: Vec<GameView>,
}

/// "Buscar informações da biblioteca": only games never looked up (or a week-old miss).
#[tauri::command]
async fn fetch_meta_all(app: AppHandle) -> Result<MetaAll, String> {
    let (keys, dir, todo) = {
        let state = app.state::<AppState>();
        let keys = state.settings.lock().unwrap().keys();
        if !keys.any() {
            return Err("Coloque sua chave da SteamGridDB ou da IGDB em Configurações primeiro.".into());
        }
        let now = library::now();
        let todo: Vec<(String, String)> = get_library(app.clone())
            .into_iter()
            .filter(|g| g.in_library)
            .filter_map(|g| {
                let cached = meta::load(&state.meta_dir, &g.id);
                meta::stale(cached.as_ref(), now)
                    .then(|| (g.id.clone(), cached.map(|m| m.term).filter(|t| !t.is_empty()).unwrap_or(g.title)))
            })
            .collect();
        (keys, state.meta_dir.clone(), todo)
    };
    let mut found = 0;
    for (id, term) in &todo {
        if meta::fetch(&dir, id, term, &keys).await.found() {
            found += 1;
        }
    }
    Ok(MetaAll { looked_up: todo.len(), found, library: get_library(app.clone()) })
}

/// An IGDB game page saved in the cache by a lookup.
fn known_igdb_page(state: &AppState, url: &str) -> bool {
    url.starts_with("https://www.igdb.com/games/")
        && state
            .library
            .lock()
            .unwrap()
            .games
            .keys()
            .any(|id| meta::load(&state.meta_dir, id).and_then(|m| m.igdb_url).as_deref() == Some(url))
}

#[tauri::command]
fn set_settings(app: AppHandle, close_to_tray: bool, show_hidden: bool) -> Result<AppSettings, String> {
    {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        s.close_to_tray = close_to_tray;
        s.show_hidden = show_hidden;
        s.save(&state.settings_path)?;
    }
    Ok(get_settings(app))
}

/// "Modo tela cheia" (the Big Picture-style library).
#[tauri::command]
fn set_fullscreen(app: AppHandle, on: bool) -> Result<(), String> {
    let w = app.get_webview_window("main").ok_or("Janela não encontrada.")?;
    w.set_fullscreen(on).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct AppUpdate {
    version: String,
    url: String,
}

/// RecompHUB's own releases, as Ports Launcher checks its GitHub page. While the repository
/// is private the API answers 404 and nothing is shown.
#[tauri::command]
async fn check_app_update() -> Option<AppUpdate> {
    #[derive(serde::Deserialize)]
    struct Rel {
        tag_name: String,
        html_url: String,
    }
    let resp = github::client()
        .get(format!("https://api.github.com/repos/{APP_REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    let rel: Rel = resp.error_for_status().ok()?.json().await.ok()?;
    let newer = version_tuple(&rel.tag_name)? > version_tuple(env!("CARGO_PKG_VERSION"))?;
    let page = format!("https://github.com/{APP_REPO}/releases/");
    (newer && rel.html_url.starts_with(&page)).then(|| AppUpdate { version: rel.tag_name, url: rel.html_url })
}

const APP_REPO: &str = "genriquesento-ops/recomphub";

/// "v0.7.0" → (0, 7, 0).
fn version_tuple(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next().flatten().unwrap_or(0)))
}

#[tauri::command]
fn open_app_update(app: AppHandle, url: String) -> Result<(), String> {
    if !url.starts_with(&format!("https://github.com/{APP_REPO}/releases/")) {
        return Err("Link desconhecido.".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

// ---- Aba Comunidade ----

#[derive(Serialize)]
struct CommunityItemView {
    #[serde(flatten)]
    item: community::Item,
    /// Same project in RecompHUB's verified catalog: its id.
    catalog_id: Option<String>,
    /// Already in the library as a community port: its id.
    installed_id: Option<String>,
}

#[derive(Serialize)]
struct CommunityView {
    items: Vec<CommunityItemView>,
    fetched_at: u64,
    errors: Vec<String>,
    list_page: &'static str,
}

/// The catalog port for a repository, also when the project moved (same repository name).
fn catalog_for_repo(repo: &str) -> Option<&'static GameDef> {
    let name = |r: &str| r.split('/').nth(1).unwrap_or("").to_lowercase();
    catalog::games()
        .iter()
        .find(|g| g.repo.eq_ignore_ascii_case(repo))
        .or_else(|| catalog::games().iter().find(|g| name(&g.repo) == name(repo)))
}

#[tauri::command]
async fn get_community(app: AppHandle, force: bool) -> CommunityView {
    let path = app.state::<AppState>().community_path.clone();
    let cache = community::refresh(&path, force).await;
    let state = app.state::<AppState>();
    let lib = state.library.lock().unwrap();
    let items = cache
        .items
        .into_iter()
        .map(|item| {
            let id = community::id_for(&item.repo);
            CommunityItemView {
                catalog_id: catalog_for_repo(&item.repo).map(|g| g.id.clone()),
                installed_id: lib.games.contains_key(&id).then_some(id),
                item,
            }
        })
        .collect();
    CommunityView { items, fetched_at: cache.fetched_at, errors: cache.errors, list_page: community::LIST_PAGE }
}

/// A repository the page may act on: listed in the Comunidade cache or installed from it.
fn known_community_repo(state: &AppState, repo: &str) -> bool {
    community::valid_repo(repo)
        && (community::load(&state.community_path).items.iter().any(|i| i.repo.eq_ignore_ascii_case(repo))
            || state.library.lock().unwrap().games.values().any(|e| e.repo.as_deref() == Some(repo)))
}

#[tauri::command]
async fn community_release(app: AppHandle, repo: String) -> Result<Option<community::CommunityRelease>, String> {
    if !known_community_repo(&app.state::<AppState>(), &repo) {
        return Err("Port fora da lista da Comunidade.".into());
    }
    community::latest_release(&repo).await
}

/// Files kept when a community port is updated or uninstalled (common save/config places).
// Windows folder names ignore case, so one spelling each.
const COMMUNITY_KEEP: &[&str] = &["portable.txt", "saves/", "save/", "savedata/", "user/", "config/"];

/// "Instalar (experimental)": a community port's Windows .zip from its latest release.
/// `asset` None picks the most likely Windows file (used by "Atualizar").
#[tauri::command]
async fn install_community(app: AppHandle, repo: String, asset: Option<String>, title: String) -> Result<GameView, String> {
    if !known_community_repo(&app.state::<AppState>(), &repo) {
        return Err("Port fora da lista da Comunidade.".into());
    }
    let id = community::id_for(&repo);
    {
        let state = app.state::<AppState>();
        let s = state.sessions.lock().unwrap();
        if s.is_running(&id) || s.is_launching(&id) {
            return Err("Feche o jogo antes de atualizar.".into());
        }
        drop(s);
        if !state.installing.lock().unwrap().insert(id.clone()) {
            return Err("A instalação já está em andamento.".into());
        }
    }
    let result = install_community_inner(&app, &id, &repo, asset, &title).await;
    let state = app.state::<AppState>();
    state.installing.lock().unwrap().remove(&id);
    result?;
    tray::refresh(&app);
    view_any(&state, &id)
}

async fn install_community_inner(app: &AppHandle, id: &str, repo: &str, asset: Option<String>, title: &str) -> Result<(), String> {
    let asset = match asset {
        Some(a) => a,
        None => community::latest_release(repo)
            .await?
            .and_then(|r| r.assets.into_iter().next())
            .map(|a| a.name)
            .ok_or("Este port não tem mais uma versão para Windows.")?,
    };
    let release = community::release_for(repo, &asset).await?;
    let state = app.state::<AppState>();
    let target = state.games_root.join("community").join(id);
    let keep: Vec<String> = COMMUNITY_KEEP.iter().map(|s| s.to_string()).collect();
    install::install_release(app, id, &release, &target, &keep).await?;
    saves::restore(&state.pending_root.join(id), &target)?;
    let exes = community::find_executables(&target);
    if exes.is_empty() {
        return Err("O arquivo baixado não tem nenhum programa (.exe). Talvez este port precise ser compilado: veja a página do projeto.".into());
    }
    let exe = (exes.len() == 1).then(|| exes[0].clone());
    let title = if title.trim().is_empty() { repo.split('/').nth(1).unwrap_or(repo) } else { title.trim() };
    {
        let mut lib = state.library.lock().unwrap();
        lib.add_community(id, &target, repo, title, &release.tag, exe);
    }
    state.latest.lock().unwrap().insert(id.to_string(), release);
    save(&state)
}

/// Programs a community port's Play button can start ("Escolher programa").
#[tauri::command]
fn list_executables(app: AppHandle, id: String) -> Result<Vec<String>, String> {
    let dir = app.state::<AppState>().library.lock().unwrap().install_dir(&id).ok_or("Jogo fora da biblioteca.")?;
    Ok(community::find_executables(&dir))
}

#[tauri::command]
fn set_executable(app: AppHandle, id: String, exe: String) -> Result<GameView, String> {
    let dir = app.state::<AppState>().library.lock().unwrap().install_dir(&id).ok_or("Jogo fora da biblioteca.")?;
    if !community::valid_exe(&dir, &exe) {
        return Err("Escolha um programa .exe de dentro da pasta do port.".into());
    }
    update_entry(&app, &id, |e| e.exe = Some(exe))
}

#[derive(Serialize)]
struct VersionInfo {
    tag: String,
    published: u64,
    size: u64,
}

/// "Escolher versão": the last releases with a Windows build.
#[tauri::command]
async fn list_versions(id: String) -> Result<Vec<VersionInfo>, String> {
    let game = game_or_err(&id)?;
    Ok(github::releases(game, 8)
        .await?
        .into_iter()
        .map(|r| VersionInfo { tag: r.tag, published: r.published, size: r.asset_size })
        .collect())
}

#[tauri::command]
fn set_ignore_updates(app: AppHandle, id: String, ignore: bool) -> Result<GameView, String> {
    let state = app.state::<AppState>();
    state.library.lock().unwrap().games.get_mut(&id).ok_or("Jogo fora da biblioteca.")?.ignore_updates = ignore;
    save(&state)?;
    view_any(&state, &id)
}

#[tauri::command]
fn reset_playtime(app: AppHandle, id: String) -> Result<GameView, String> {
    let state = app.state::<AppState>();
    {
        let mut lib = state.library.lock().unwrap();
        let e = lib.games.get_mut(&id).ok_or("Jogo fora da biblioteca.")?;
        e.total_seconds = 0;
        e.last_played = None;
        e.play_count = 0;
    }
    save(&state)?;
    {
        let mut history = state.history.lock().unwrap();
        let profile = state.profiles.lock().unwrap().active.clone();
        history.sessions.retain(|r| r.id != id || r.profile != profile);
        history.save(&state.history_path)?;
    }
    view_any(&state, &id)
}

/// Deletes a port RecompHUB installed, keeping the player's files for the next install.
/// A port added from the player's own folder is never deleted.
#[tauri::command]
fn uninstall_game(app: AppHandle, id: String) -> Result<Vec<String>, String> {
    let game = catalog::get(&id);
    let state = app.state::<AppState>();
    let entry = state.library.lock().unwrap().games.get(&id).cloned().ok_or("Jogo fora da biblioteca.")?;
    let dir = PathBuf::from(&entry.install_dir);
    if entry.version.is_none() || !dir.starts_with(&state.games_root) || dir == state.games_root {
        return Err("Este port foi adicionado de uma pasta sua: o RecompHUB não apaga arquivos que não instalou. Use \"Remover da biblioteca\".".into());
    }
    {
        let s = state.sessions.lock().unwrap();
        if s.is_running(&id) || s.is_launching(&id) {
            return Err("Feche o jogo antes de desinstalar.".into());
        }
    }
    if state.installing.lock().unwrap().contains(&id) {
        return Err("Espere a instalação terminar.".into());
    }
    let owned = match game {
        Some(game) => saves::owned_paths(&dir, &game.keep_on_update, files::save_dir(game, &dir).as_deref()),
        None => {
            let keep: Vec<String> = COMMUNITY_KEEP.iter().map(|s| s.to_string()).collect();
            saves::owned_paths(&dir, &keep, None)
        }
    };
    let kept = saves::preserve(&dir, &owned, &state.pending_root.join(&id))?;
    std::fs::remove_dir_all(&dir).map_err(|e| format!("Não consegui apagar {}: {e}", dir.display()))?;
    state.library.lock().unwrap().remove(&id);
    save(&state)?;
    Ok(kept)
}

#[derive(Serialize)]
struct BackupResult {
    path: String,
    games: usize,
    files: usize,
}

/// "Backup dos saves": every port's saves in one dated .zip.
#[tauri::command]
async fn backup_saves(app: AppHandle) -> Result<BackupResult, String> {
    let (items, folder) = {
        let state = app.state::<AppState>();
        let lib = state.library.lock().unwrap();
        let items: Vec<(String, PathBuf)> = lib
            .games
            .iter()
            .filter_map(|(id, e)| {
                let game = catalog::get(id)?;
                Some((id.clone(), files::save_dir(game, Path::new(&e.install_dir))?))
            })
            .collect();
        (items, state.backups_root.clone())
    };
    if items.is_empty() {
        return Err("Nenhum port da biblioteca tem saves ainda.".into());
    }
    let games = items.len();
    let date = library::date(library::now());
    let (path, files) = tauri::async_runtime::spawn_blocking(move || saves::backup(&items, &folder, &date))
        .await
        .map_err(|e| e.to_string())??;
    Ok(BackupResult { path: path.display().to_string(), games, files })
}

/// Folders whose contents count as the game's saves, labelled for the backup .zip.
/// Catalog port: the port's save folder. Community port: its save folders inside the port.
/// PC game: none (Ludusavi knows them).
pub(crate) fn save_sources(state: &AppState, id: &str) -> Vec<(String, PathBuf)> {
    let Some(entry) = state.library.lock().unwrap().games.get(id).cloned() else { return vec![] };
    let dir = PathBuf::from(&entry.install_dir);
    match entry.kind {
        library::EntryKind::Port => catalog::get(id)
            .and_then(|game| files::save_dir(game, &dir))
            .map(|d| vec![("saves".to_string(), d)])
            .unwrap_or_default(),
        library::EntryKind::Community => ["saves", "save", "savedata", "user"]
            .iter()
            .map(|n| (n.to_string(), dir.join(n)))
            .filter(|(_, p)| p.is_dir())
            .collect(),
        library::EntryKind::PcGame => vec![],
    }
}

#[derive(Serialize, Clone)]
struct BackupEvent {
    id: String,
    title: String,
    /// "created", "unchanged", "ludusavi" or "error"
    result: &'static str,
    message: String,
}

/// After a session: a backup of the saves (when they changed), and for PC games, Ludusavi's
/// backup when the player turned it on. Runs on its own thread.
pub(crate) fn backup_after_session(app: AppHandle, id: String) {
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap().clone();
        let Ok(view) = view_any(&state, &id) else { return };
        let title = view.title.clone();
        let emit = |result: &'static str, message: String| {
            let _ = app.emit("backup", BackupEvent { id: id.clone(), title: title.clone(), result, message });
        };
        let sources = save_sources(&state, &id);
        if settings.auto_backup && !sources.is_empty() {
            match autosave::make(&state.auto_root, &id, &sources, "ao fechar o jogo", settings.backup_keep as usize, library::now()) {
                Ok(autosave::Made::Created(_)) => emit("created", format!("Backup dos saves de {title} feito.")),
                Ok(_) => {}
                Err(e) => emit("error", format!("{title}: {e}")),
            }
        }
        if settings.ludusavi_auto && view.kind == "pc_game" {
            if let Some(exe) = ludusavi::locate(&settings.ludusavi_path) {
                match ludusavi_backup_now(&state, &id, &exe) {
                    Ok(o) if o.processed => emit("ludusavi", format!("Ludusavi: backup de {} ({} arquivo(s)).", o.name, o.files)),
                    Ok(_) => {}
                    Err(e) => emit("error", format!("Ludusavi, {title}: {e}")),
                }
            }
        }
    });
}

fn ludusavi_backup_now(state: &AppState, id: &str, exe: &Path) -> Result<ludusavi::Outcome, String> {
    let (known, title) = {
        let lib = state.library.lock().unwrap();
        let e = lib.games.get(id).ok_or("Jogo fora da biblioteca.")?;
        (e.ludusavi_name.clone(), e.title.clone().unwrap_or_default())
    };
    let name = match known {
        Some(n) => n,
        None => {
            let n = ludusavi::find(exe, &title)?.ok_or_else(|| {
                format!("o Ludusavi não conhece \"{title}\". Escreva o nome como o Ludusavi chama o jogo na página do jogo.")
            })?;
            if let Some(e) = state.library.lock().unwrap().games.get_mut(id) {
                e.ludusavi_name = Some(n.clone());
            }
            save(state)?;
            n
        }
    };
    ludusavi::backup(exe, &name)
}

#[derive(Serialize)]
struct GameBackups {
    backups: Vec<autosave::BackupInfo>,
    /// Where the saves are (label, folder).
    sources: Vec<(String, String)>,
    last_change: Option<u64>,
    keep: u32,
    auto_backup: bool,
    /// PC game: Ludusavi found on this PC, and the name it knows the game by.
    ludusavi: Option<String>,
    ludusavi_name: Option<String>,
}

#[tauri::command]
async fn get_backups(app: AppHandle, id: String) -> Result<GameBackups, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().unwrap().clone();
        let sources = save_sources(&state, &id);
        let ludusavi_name = state.library.lock().unwrap().games.get(&id).and_then(|e| e.ludusavi_name.clone());
        GameBackups {
            backups: autosave::list(&state.auto_root, &id),
            last_change: autosave::newest_change(&sources),
            sources: sources.iter().map(|(l, p)| (l.clone(), p.display().to_string())).collect(),
            keep: settings.backup_keep,
            auto_backup: settings.auto_backup,
            ludusavi: ludusavi::locate(&settings.ludusavi_path).map(|p| p.display().to_string()),
            ludusavi_name,
        }
    })
    .await
    .map_err(|e| e.to_string())
}

fn not_running(state: &AppState, id: &str) -> Result<(), String> {
    let s = state.sessions.lock().unwrap();
    if s.is_running(id) || s.is_launching(id) {
        return Err("Feche o jogo primeiro.".into());
    }
    Ok(())
}

/// "Fazer backup agora".
#[tauri::command]
async fn backup_now(app: AppHandle, id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let sources = save_sources(&state, &id);
        if sources.is_empty() {
            return Err("O RecompHUB não sabe onde este jogo guarda os saves.".to_string());
        }
        let keep = state.settings.lock().unwrap().backup_keep as usize;
        match autosave::make(&state.auto_root, &id, &sources, "manual", keep, library::now())? {
            autosave::Made::Created(b) => Ok(format!("Backup feito: {} arquivo(s).", b.files)),
            autosave::Made::Unchanged => Ok("Os saves não mudaram desde o último backup.".to_string()),
            autosave::Made::Empty => Err("Ainda não há saves deste jogo.".to_string()),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Puts a backup back, after backing up the saves as they are now.
#[tauri::command]
async fn restore_backup(app: AppHandle, id: String, file: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        not_running(&state, &id)?;
        let sources = save_sources(&state, &id);
        if sources.is_empty() {
            return Err("O RecompHUB não sabe onde este jogo guarda os saves.".to_string());
        }
        let keep = state.settings.lock().unwrap().backup_keep as usize;
        // Nothing is trimmed until the restore is done, so the chosen backup cannot go away.
        autosave::make(&state.auto_root, &id, &sources, "antes de restaurar", usize::MAX, library::now())?;
        let n = autosave::restore(&state.auto_root, &id, &file, &sources)?;
        autosave::trim(&state.auto_root, &id, keep);
        Ok(format!("Backup restaurado ({n} arquivo(s)). Os saves de antes ficaram num backup \"antes de restaurar\"."))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn set_backup_locked(app: AppHandle, id: String, file: String, locked: bool) -> Result<(), String> {
    autosave::set_locked(&app.state::<AppState>().auto_root, &id, &file, locked)
}

#[tauri::command]
fn open_game_backups(app: AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    if !state.library.lock().unwrap().games.contains_key(&id) {
        return Err("Jogo fora da biblioteca.".into());
    }
    let folder = state.auto_root.join(&id);
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    app.opener().open_path(folder.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct BackupSettings {
    auto_backup: bool,
    backup_keep: u32,
    ludusavi_auto: bool,
    /// Where ludusavi.exe was found, if anywhere.
    ludusavi: Option<String>,
    ludusavi_page: &'static str,
}

#[tauri::command]
fn get_backup_settings(app: AppHandle) -> BackupSettings {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    BackupSettings {
        auto_backup: s.auto_backup,
        backup_keep: s.backup_keep,
        ludusavi_auto: s.ludusavi_auto,
        ludusavi: ludusavi::locate(&s.ludusavi_path).map(|p| p.display().to_string()),
        ludusavi_page: ludusavi::PROJECT_PAGE,
    }
}

#[tauri::command]
fn set_backup_settings(app: AppHandle, auto_backup: bool, backup_keep: u32, ludusavi_auto: bool) -> Result<BackupSettings, String> {
    {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        s.auto_backup = auto_backup;
        s.backup_keep = backup_keep.clamp(1, 100);
        s.ludusavi_auto = ludusavi_auto;
        s.save(&state.settings_path)?;
    }
    Ok(get_backup_settings(app))
}

/// "Escolher o ludusavi.exe".
#[tauri::command]
async fn choose_ludusavi(app: AppHandle) -> Result<Option<BackupSettings>, String> {
    let picked = app
        .dialog()
        .file()
        .set_title("Escolha o ludusavi.exe")
        .add_filter("Ludusavi", &["exe"])
        .blocking_pick_file()
        .and_then(|p| p.into_path().ok());
    let Some(path) = picked else { return Ok(None) };
    if !path.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case("ludusavi.exe")).unwrap_or(false) {
        return Err("Escolha o arquivo ludusavi.exe.".into());
    }
    {
        let state = app.state::<AppState>();
        let mut s = state.settings.lock().unwrap();
        s.ludusavi_path = path.display().to_string();
        s.save(&state.settings_path)?;
    }
    Ok(Some(get_backup_settings(app)))
}

/// PC game: the name Ludusavi uses ("" forgets it, so it is looked up again).
#[tauri::command]
fn set_ludusavi_name(app: AppHandle, id: String, name: String) -> Result<GameView, String> {
    let name = name.trim().to_string();
    if name.len() > 200 {
        return Err("Nome longo demais.".into());
    }
    update_entry(&app, &id, |e| e.ludusavi_name = Some(name).filter(|n| !n.is_empty()))
}

#[tauri::command]
async fn ludusavi_backup(app: AppHandle, id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let path = state.settings.lock().unwrap().ludusavi_path.clone();
        let exe = ludusavi::locate(&path).ok_or("Não encontrei o Ludusavi neste PC.")?;
        let o = ludusavi_backup_now(&state, &id, &exe)?;
        if o.failed {
            Ok(format!("Ludusavi: backup de {} com falhas em alguns arquivos.", o.name))
        } else if o.processed {
            Ok(format!("Ludusavi: backup de {} feito ({} arquivo(s)), na pasta de backups do Ludusavi.", o.name, o.files))
        } else {
            Ok(format!("Ludusavi: nenhum save de {} encontrado neste PC.", o.name))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---- Perfis ----

#[derive(Serialize)]
struct ProfilesView {
    active: profiles::Profile,
    list: Vec<profiles::Profile>,
    ask_on_start: bool,
    separate_saves: Vec<String>,
    colors: &'static [&'static str],
}

fn profiles_view(state: &AppState) -> ProfilesView {
    let p = state.profiles.lock().unwrap();
    ProfilesView {
        active: p.active().clone(),
        list: p.list.clone(),
        ask_on_start: p.ask_on_start,
        separate_saves: p.separate_saves.clone(),
        colors: profiles::COLORS,
    }
}

fn clean_color(color: &str) -> String {
    profiles::COLORS.iter().find(|c| **c == color).unwrap_or(&profiles::COLORS[0]).to_string()
}

#[tauri::command]
fn get_profiles(app: AppHandle) -> ProfilesView {
    profiles_view(&app.state::<AppState>())
}

#[tauri::command]
fn create_profile(app: AppHandle, name: String, color: String) -> Result<ProfilesView, String> {
    let name = profiles::clean_name(&name)?;
    let state = app.state::<AppState>();
    {
        let mut p = state.profiles.lock().unwrap();
        if p.list.len() >= 12 {
            return Err("O RecompHUB guarda até 12 perfis.".into());
        }
        let id = profiles::new_id(&p, &name, library::now());
        p.list.push(profiles::Profile { id, name, color: clean_color(&color), created: library::now() });
        p.save(&state.profiles_path)?;
    }
    Ok(profiles_view(&state))
}

#[tauri::command]
fn edit_profile(app: AppHandle, id: String, name: String, color: String) -> Result<ProfilesView, String> {
    let name = profiles::clean_name(&name)?;
    let state = app.state::<AppState>();
    {
        let mut p = state.profiles.lock().unwrap();
        let profile = p.list.iter_mut().find(|x| x.id == id).ok_or("Perfil não encontrado.")?;
        profile.name = name;
        profile.color = clean_color(&color);
        p.save(&state.profiles_path)?;
    }
    tray::refresh(&app);
    Ok(profiles_view(&state))
}

/// Deletes a profile that is not in use: its progress, history and waiting saves.
#[tauri::command]
fn delete_profile(app: AppHandle, id: String) -> Result<ProfilesView, String> {
    let state = app.state::<AppState>();
    {
        let mut p = state.profiles.lock().unwrap();
        if p.active == id {
            return Err("Troque para outro perfil antes de apagar este.".into());
        }
        if p.get(&id).is_none() {
            return Err("Perfil não encontrado.".into());
        }
        p.list.retain(|x| x.id != id);
        p.save(&state.profiles_path)?;
    }
    let _ = std::fs::remove_file(state.profiles_dir.join(format!("{id}.json")));
    let _ = std::fs::remove_dir_all(state.profiles_dir.join("saves").join(&id));
    {
        let mut h = state.history.lock().unwrap();
        h.sessions.retain(|r| r.profile != id);
        h.save(&state.history_path)?;
    }
    Ok(profiles_view(&state))
}

#[tauri::command]
fn set_profile_options(app: AppHandle, ask_on_start: bool) -> Result<ProfilesView, String> {
    let state = app.state::<AppState>();
    {
        let mut p = state.profiles.lock().unwrap();
        p.ask_on_start = ask_on_start;
        p.save(&state.profiles_path)?;
    }
    Ok(profiles_view(&state))
}

fn nothing_running(state: &AppState) -> Result<(), String> {
    if !state.sessions.lock().unwrap().running.is_empty() {
        return Err("Feche os jogos abertos antes de trocar de perfil.".into());
    }
    if !state.installing.lock().unwrap().is_empty() {
        return Err("Espere a instalação terminar antes de trocar de perfil.".into());
    }
    Ok(())
}

/// Moves everything inside `from` into `to` (created if needed), leaving `from` empty.
fn move_contents(from: &Path, to: &Path) -> Result<(), String> {
    let Ok(entries) = std::fs::read_dir(from) else { return Ok(()) };
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for e in entries.flatten() {
        let dst = to.join(e.file_name());
        if dst.exists() {
            let _ = if dst.is_dir() { std::fs::remove_dir_all(&dst) } else { std::fs::remove_file(&dst) };
        }
        saves::move_tree(&e.path(), &dst).map_err(|err| format!("Não consegui mover {}: {err}", e.path().display()))?;
    }
    Ok(())
}

/// Games with "saves separados por perfil": a backup first, then the leaving profile's saves
/// go to its waiting folder and the arriving profile's come back (none: an empty folder).
/// The save folders themselves stay, so the ports keep finding them.
fn swap_saves(state: &AppState, from: &str, to: &str, to_name: &str) -> Result<(), String> {
    let games = state.profiles.lock().unwrap().separate_saves.clone();
    let keep = state.settings.lock().unwrap().backup_keep as usize;
    for id in games {
        let sources = save_sources(state, &id);
        if sources.is_empty() {
            continue;
        }
        autosave::make(&state.auto_root, &id, &sources, &format!("antes de trocar para o perfil {to_name}"), usize::MAX, library::now())?;
        autosave::trim(&state.auto_root, &id, keep);
        for (label, folder) in &sources {
            move_contents(folder, &profiles::stash(&state.profiles_dir, from, &id, label))?;
            let waiting = profiles::stash(&state.profiles_dir, to, &id, label);
            move_contents(&waiting, folder)?;
            let _ = std::fs::remove_dir_all(&waiting);
        }
    }
    Ok(())
}

#[tauri::command]
async fn switch_profile(app: AppHandle, id: String) -> Result<ProfilesView, String> {
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        nothing_running(&state)?;
        let (from, to_name) = {
            let p = state.profiles.lock().unwrap();
            let to = p.get(&id).ok_or("Perfil não encontrado.")?;
            (p.active.clone(), to.name.clone())
        };
        if from == id {
            return Ok(());
        }
        swap_saves(&state, &from, &id, &to_name)?;
        let mut lib = state.library.lock().unwrap();
        let mut p = state.profiles.lock().unwrap();
        profiles::switch(&mut lib, &mut p, &state.profiles_dir, &id)?;
        lib.save(&state.library_path)?;
        p.save(&state.profiles_path)
    })
    .await
    .map_err(|e| e.to_string())??;
    tray::refresh(&app);
    Ok(profiles_view(&app.state::<AppState>()))
}

/// "Saves separados por perfil" for one game. Turning it on: the saves there now belong to
/// the active profile; other profiles start with no saves. Turning it off: the active
/// profile's saves stay; the other profiles' wait in their folders.
#[tauri::command]
fn set_separate_saves(app: AppHandle, id: String, on: bool) -> Result<ProfilesView, String> {
    let state = app.state::<AppState>();
    not_running(&state, &id)?;
    if on && save_sources(&state, &id).is_empty() {
        return Err("Ainda não há pasta de saves deste jogo. Jogue e salve uma vez antes.".into());
    }
    {
        let mut p = state.profiles.lock().unwrap();
        p.separate_saves.retain(|g| *g != id);
        if on {
            p.separate_saves.push(id);
        }
        p.save(&state.profiles_path)?;
    }
    Ok(profiles_view(&state))
}

#[tauri::command]
fn open_backups(app: AppHandle) -> Result<(), String> {
    let folder = app.state::<AppState>().backups_root.clone();
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    app.opener().open_path(folder.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

/// The player picks their game files (or dropped them on the window, or accepted the one
/// found in Downloads: `path`); they are checked and handed to the port.
/// Ok(None) when the player closed the dialog.
#[tauri::command]
async fn provide_files(app: AppHandle, id: String, path: Option<String>) -> Result<Option<FilesStatus>, String> {
    let game = game_or_err(&id)?;
    let dir = app
        .state::<AppState>()
        .library
        .lock()
        .unwrap()
        .install_dir(game.id.as_str())
        .ok_or("Instale o jogo primeiro.")?;
    let picked = if let Some(p) = path.filter(|p| !p.trim().is_empty()) {
        Some(PathBuf::from(p))
    } else if game.files_kind() == "folder" {
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

/// "Procurar na pasta Downloads": only when the player clicks, only ROM files, and only an
/// exact match comes back. The player still confirms before it is handed to the port.
#[tauri::command]
async fn find_rom_in_downloads(app: AppHandle, id: String) -> Result<Option<String>, String> {
    let game = game_or_err(&id)?;
    let downloads = app.path().download_dir().map_err(|_| "Não encontrei a pasta Downloads.".to_string())?;
    let found = tauri::async_runtime::spawn_blocking(move || files::find_rom(game, &downloads))
        .await
        .map_err(|e| e.to_string())?;
    Ok(found.map(|p| p.display().to_string()))
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
    play::start(&app, &id, &dir, &exe, &entry.launch_args, &entry.env)
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
            let icon = Some(dir.join(&game.icon)).filter(|p| p.is_file()).unwrap_or_else(|| dir.join(&game.exe));
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

/// A game chosen in the tray menu.
pub fn play_from_tray(app: &AppHandle, id: &str) -> Result<(), String> {
    play(app.clone(), id.to_string())
}

/// A message for the page's toast (from the tray or the background).
pub fn notify(app: &AppHandle, message: &str) {
    let _ = app.emit("notify", message);
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
    app.opener().open_url(&game.project_url, None::<&str>).map_err(|e| e.to_string())
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
            tray::show_window(app);
            handle_args(app, &args);
        }))
        // Closing the window keeps RecompHUB in the tray (setting "Fechar para a bandeja").
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let to_tray = window.app_handle().state::<AppState>().settings.lock().unwrap().close_to_tray;
                if to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            catalog_sync::load_startup(&dir);
            let local = app.path().app_local_data_dir()?;
            let library_path = dir.join("library.json");
            let settings_path = dir.join("settings.json");
            app.manage(AppState {
                settings: Mutex::new(covers::Settings::load(&settings_path)),
                settings_path,
                covers: covers::Dirs::new(&dir),
                games_root: local.join("games"),
                pending_root: local.join("saves-pendentes"),
                backups_root: dir.join("Backups de saves"),
                auto_root: dir.join("Backups de saves").join("automaticos"),
                community_path: dir.join("community.json"),
                meta_dir: dir.join("meta"),
                history: Mutex::new(history::History::load(&dir.join("history.json"))),
                history_path: dir.join("history.json"),
                profiles: Mutex::new(profiles::Profiles::load(&dir.join("profiles.json"))),
                profiles_path: dir.join("profiles.json"),
                profiles_dir: dir.join("profiles"),
                library: Mutex::new(Library::load(&library_path)),
                library_path,
                latest: Mutex::new(HashMap::new()),
                sessions: Mutex::new(play::Sessions::default()),
                installing: Mutex::new(HashSet::new()),
                pending_open: Mutex::new(None),
            });
            play::spawn_watcher(app.handle().clone());
            tray::build(app.handle())?;
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
            find_rom_in_downloads,
            set_api_keys,
            game_meta,
            fetch_meta_all,
            set_completion,
            set_notes,
            get_history,
            get_stats,
            get_backups,
            backup_now,
            restore_backup,
            set_backup_locked,
            open_game_backups,
            get_backup_settings,
            set_backup_settings,
            choose_ludusavi,
            set_ludusavi_name,
            ludusavi_backup,
            get_profiles,
            create_profile,
            edit_profile,
            delete_profile,
            set_profile_options,
            switch_profile,
            set_separate_saves,
            play,
            stop_game,
            open_saves,
            create_shortcut,
            add_pc_game,
            rename_game,
            set_tags,
            open_link,
            list_versions,
            get_community,
            community_release,
            install_community,
            list_executables,
            set_executable,
            set_ignore_updates,
            reset_playtime,
            uninstall_game,
            backup_saves,
            open_backups,
            choose_cover,
            clear_cover,
            download_covers,
            get_cover_templates,
            set_cover_templates,
            set_favorite,
            set_hidden,
            set_launch_options,
            get_settings,
            set_settings,
            set_fullscreen,
            check_app_update,
            open_app_update,
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

#[cfg(test)]
mod tests {
    #[test]
    fn versions_compare_by_number() {
        assert_eq!(super::version_tuple("v0.10.2"), Some((0, 10, 2)));
        assert_eq!(super::version_tuple("1.2"), Some((1, 2, 0)));
        assert!(super::version_tuple("v0.10.0") > super::version_tuple("0.9.9"));
        assert_eq!(super::version_tuple("latest"), None);
    }
}

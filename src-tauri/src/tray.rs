//! The tray icon, as Ports Launcher's: left click shows RecompHUB; right click lists the
//! five most recently played games (straight to Play), "Abrir RecompHUB" and "Sair".
//! Closing the window keeps RecompHUB here, so play time is still counted.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::AppState;

const TRAY_ID: &str = "main";

pub fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// (id, title) of the games played most recently, newest first.
fn recent(app: &AppHandle) -> Vec<(String, String)> {
    let state = app.state::<AppState>();
    let lib = state.library.lock().unwrap();
    let mut played: Vec<_> = lib.games.iter().filter(|(_, e)| e.last_played.is_some() && !e.hidden).collect();
    played.sort_by_key(|(_, e)| std::cmp::Reverse(e.last_played));
    played
        .into_iter()
        .take(5)
        .map(|(id, e)| {
            let title = crate::catalog::get(id)
                .map(|g| g.title.clone())
                .or_else(|| e.title.clone())
                .unwrap_or_else(|| id.clone());
            (id.clone(), title)
        })
        .collect()
}

fn menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::new(app)?;
    let games = recent(app);
    if games.is_empty() {
        menu.append(&MenuItem::with_id(app, "none", "Nenhum jogo recente", false, None::<&str>)?)?;
    }
    for (id, title) in games {
        menu.append(&MenuItem::with_id(app, format!("play:{id}"), format!("Jogar {title}"), true, None::<&str>)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "open", "Abrir RecompHUB", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?)?;
    Ok(menu)
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("RecompHUB")
        .menu(&menu(app)?)
        .menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref().to_string();
            match id.as_str() {
                "open" => show_window(app),
                "quit" => app.exit(0),
                other => {
                    if let Some(game) = other.strip_prefix("play:") {
                        if let Err(e) = crate::play_from_tray(app, game) {
                            show_window(app);
                            crate::notify(app, &e);
                        }
                    }
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Rebuilds the recent-games list (after a session or a library change).
pub fn refresh(app: &AppHandle) {
    if let (Some(tray), Ok(m)) = (app.tray_by_id(TRAY_ID), menu(app)) {
        let _ = tray.set_menu(Some(m));
    }
}

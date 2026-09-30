//! System tray — Vara lives in the taskbar even when the window is closed.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Vara — attentive", false, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let show = MenuItem::with_id(app, "show", "Show / Hide window", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause / Resume entity", true, None::<&str>)?;
    let data = MenuItem::with_id(app, "data", "Open data folder", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Vara", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&status, &sep1, &show, &pause, &data, &sep2, &quit])?;

    let mut builder = TrayIconBuilder::with_id("vara-tray")
        .tooltip("Vara — attentive")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => toggle_main(app),
            "pause" => toggle_pause(app),
            "data" => open_data_dir(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn toggle_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let visible = w.is_visible().unwrap_or(false);
        let focused = w.is_focused().unwrap_or(false);
        if visible && focused {
            let _ = w.hide();
        } else {
            show_main(app);
        }
    }
}

fn toggle_pause(app: &AppHandle) {
    use std::sync::atomic::Ordering;
    if let Some(st) = app.try_state::<crate::AppState>() {
        let now = !st.paused.load(Ordering::SeqCst);
        st.paused.store(now, Ordering::SeqCst);
        let msg = if now { "Vara paused" } else { "Vara resumed" };
        let _ = st.db.insert_event("info", "pause", msg);
        crate::notify_user(app, "Vara", msg);
    }
}

fn open_data_dir(app: &AppHandle) {
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = open::that(dir);
    }
}

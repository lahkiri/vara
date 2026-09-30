//! Watched-folder indexing: drop .md/.txt files into a folder and Vara
//! remembers them (with file:// provenance). The "works with your system"
//! surface of the entity.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use vara_core::types::Settings;
use vara_core::{EntityEvent, EventSink};

pub fn start(app: &AppHandle, path: std::path::PathBuf) -> Result<(), String> {
    let path_str = path.to_string_lossy().trim().to_string();
    if path_str.is_empty() {
        return Ok(()); // nothing to watch
    }
    let path = std::path::PathBuf::from(&path_str);
    if !path.is_dir() {
        return Err(format!("watch path is not a directory: {}", path.display()));
    }

    let (tx, rx) = mpsc::channel::<Result<notify::Event, notify::Error>>();
    let mut w: RecommendedWatcher =
        notify::recommended_watcher(tx).map_err(|e| format!("watcher init: {e}"))?;
    w.watch(&path, RecursiveMode::Recursive)
        .map_err(|e| format!("watcher watch: {e}"))?;

    let st = app
        .try_state::<crate::AppState>()
        .ok_or("state not ready")?;
    *st.watcher.lock().unwrap_or_else(|p| p.into_inner()) = Some(w);

    let handle = app.clone();
    std::thread::spawn(move || {
        let mut last_seen: HashMap<std::path::PathBuf, Instant> = HashMap::new();
        for res in rx {
            let Ok(ev) = res else { continue };
            if !matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                continue;
            }
            for p in ev.paths {
                let ext = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if !["md", "txt"].contains(&ext.as_str()) {
                    continue;
                }
                let now = Instant::now();
                if last_seen
                    .get(&p)
                    .map(|t| now.duration_since(*t) < Duration::from_secs(3))
                    .unwrap_or(false)
                {
                    continue; // debounce
                }
                last_seen.insert(p.clone(), now);
                if let Err(e) = index_file(&handle, &p) {
                    if let Some(st) = handle.try_state::<crate::AppState>() {
                        let _ = st.db.insert_event("warn", "watcher", &e);
                    }
                }
            }
        }
    });
    Ok(())
}

fn index_file(app: &AppHandle, p: &std::path::Path) -> Result<(), String> {
    let meta = std::fs::metadata(p).map_err(|e| format!("stat: {e}"))?;
    if meta.len() > 512_000 {
        return Ok(()); // too big to index blindly
    }
    let content = std::fs::read_to_string(p).unwrap_or_default();
    if content.trim().is_empty() {
        return Ok(());
    }
    let title = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    let body: String = content.chars().take(2000).collect();
    let url = format!("file://{}", p.display());

    let st = app
        .try_state::<crate::AppState>()
        .ok_or("state not ready")?;
    if let Ok(Some(_)) = st.db.add_note_if_new(
        "file_observation",
        &title,
        &body,
        None,
        Some(&url),
        Some(&title),
    ) {
        let _ = st
            .db
            .insert_event("info", "watcher", &format!("indexed: {title}"));
        crate::TauriSink { app: app.clone() }.emit(EntityEvent::Activity {
            kind: "watcher".into(),
            message: format!("Indexed file: {title}"),
            mission_id: None,
        });
    }
    Ok(())
}

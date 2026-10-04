//! `vara-plugins` — inspect and manage the plugin composition from a terminal.
//!
//! The plugin system is only real if it can be exercised without the GUI: a
//! headless box, CI, and this session all need to answer "what is installed,
//! what is enabled, what does it ask for, and what would load next boot?"
//!
//! Deliberately a separate tiny binary rather than a mode of the TUI, so the
//! plugin layer can be checked even when the model provider is unconfigured.

use std::path::PathBuf;
use vara_core::plugin_registry::{default_plugin_dirs, PluginRegistry};

fn data_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("VARA_DATA_DIR") {
        if !custom.trim().is_empty() {
            return PathBuf::from(custom);
        }
    }
    let base = std::env::var("LOCALAPPDATA")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join("app.vara.entity")
}

fn install_root() -> Option<PathBuf> {
    // The binary may sit in `target/debug` (development) or beside `plugins/`
    // (installed). Prefer an explicit override, then walk up looking for
    // `plugins`, which is the same folder the shipped manifests live in.
    if let Ok(root) = std::env::var("VARA_INSTALL_ROOT") {
        let p = PathBuf::from(root);
        if p.is_dir() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let mut dir = exe.parent()?.to_path_buf();
    for _ in 0..4 {
        if dir.join("plugins").is_dir() {
            return Some(dir);
        }
        dir = dir.parent()?.to_path_buf();
    }
    // Fall back to the working directory, which is the checkout in development.
    let cwd = std::env::current_dir().ok()?;
    if cwd.join("plugins").is_dir() {
        return Some(cwd);
    }
    None
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(|s| s.as_str()).unwrap_or("list");
    let dir = data_dir();
    std::fs::create_dir_all(&dir).ok();

    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(root) = install_root() {
        dirs.push(root.join("plugins"));
    }
    for extra in default_plugin_dirs(None) {
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }

    let state_path = dir.join("plugins.json");
    let mut registry = PluginRegistry::scan(&dirs, &state_path);

    match command {
        "list" | "ls" => {
            println!("Vara plugins — {} installed", registry.rows().len());
            println!(
                "  scan     : {}",
                dirs.iter()
                    .map(|d| d.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!("  decisions: {}\n", state_path.display());
            let grouped = registry.by_slot();
            for (slot, rows) in grouped {
                println!("── {slot} ──");
                for row in rows {
                    let mark = if row.enabled { "●" } else { "○" };
                    let approved = if row.approved {
                        ""
                    } else {
                        "  [needs approval]"
                    };
                    println!(
                        "  {mark} {:<24} {:<7} {:<11}{}",
                        row.id,
                        row.version,
                        row.integrity.label(),
                        approved
                    );
                    println!("      {}", row.summary);
                    if !row.asks.is_empty() {
                        println!("      asks to: {}", row.asks.join(", "));
                    }
                }
                println!();
            }
            if !registry.errors().is_empty() {
                println!("── could not be read ──");
                for (_, err) in registry.errors() {
                    println!("  ! {err}");
                }
            }
            println!("● enabled   ○ disabled");
        }
        "plan" => match registry.load_plan() {
            Ok(plan) => {
                println!("load order ({} plugins):", plan.len());
                for (i, id) in plan.iter().enumerate() {
                    println!("  {}. {id}", i + 1);
                }
            }
            Err(why) => {
                eprintln!("cannot compose a run: {why}");
                std::process::exit(1);
            }
        },
        "enable" | "disable" => {
            let id = args.get(1).unwrap_or_else(|| {
                eprintln!("usage: vara-plugins {command} <plugin-id>");
                std::process::exit(2);
            });
            let enabled = command == "enable";
            match registry.set_enabled(id, enabled) {
                Ok(()) => println!(
                    "{id} is now {}",
                    if enabled { "enabled" } else { "disabled" }
                ),
                Err(why) => {
                    eprintln!("{why}");
                    std::process::exit(1);
                }
            }
        }
        "approve" => {
            let id = args.get(1).unwrap_or_else(|| {
                eprintln!("usage: vara-plugins approve <plugin-id>");
                std::process::exit(2);
            });
            match registry.approve(id) {
                Ok(()) => println!(
                    "{id} approved — you accepted: {}",
                    registry
                        .get(id)
                        .map(|r| r.asks.join(", "))
                        .unwrap_or_default()
                ),
                Err(why) => {
                    eprintln!("{why}");
                    std::process::exit(1);
                }
            }
        }
        "hash" => {
            let path = args.get(1).unwrap_or_else(|| {
                eprintln!("usage: vara-plugins hash <plugin-folder>");
                std::process::exit(2);
            });
            let folder = PathBuf::from(path);
            match vara_core::plugin::PluginFolder::load(&folder, false) {
                Ok(found) => {
                    let mut manifest = found.manifest.clone();
                    manifest.apply_hash();
                    println!("manifest sha256 : {}", manifest.sha256);
                    println!(
                        "  paste into plugin.toml as:  sha256 = \"{}\"",
                        manifest.sha256
                    );
                    match found.content_hash() {
                        Ok(content) => {
                            println!("content sha256  : {content}");
                            println!("  (over every file in the folder — publish this one)");
                        }
                        Err(why) => eprintln!("content hash failed: {why}"),
                    }
                }
                Err(why) => {
                    eprintln!("{why}");
                    std::process::exit(1);
                }
            }
        }
        "check" => {
            // A composition with nothing enabled is a product mistake, and a bad
            // plan is a bug: both belong in CI, not in a user's first launch.
            let plan = registry.load_plan();
            let enabled = registry.rows().iter().filter(|r| r.enabled).count();
            println!("installed: {}", registry.rows().len());
            println!("enabled  : {enabled}");
            match plan {
                Ok(plan) => println!("plan     : {} plugins, ok", plan.len()),
                Err(why) => {
                    eprintln!("plan     : FAILED — {why}");
                    std::process::exit(1);
                }
            }
            // Report **every** unreadable manifest, then fail once.
            //
            // `exit(1)` used to sit inside this loop, so a repository with three
            // broken manifests reported only the first and then quit — the loop
            // never looped (*clippy: "this loop never actually loops"*). A checker
            // that hides two thirds of the problems is worse than no checker,
            // because it implies the rest are fine.
            let mut unreadable = 0usize;
            for (_, err) in registry.errors() {
                eprintln!("unreadable manifest: {err}");
                unreadable += 1;
            }
            if unreadable > 0 {
                eprintln!("{unreadable} manifest(s) could not be read");
                std::process::exit(1);
            }
        }
        other => {
            eprintln!("unknown command '{other}'. Try: list, plan, check, hash <dir>, enable <id>, disable <id>, approve <id>");
            std::process::exit(2);
        }
    }
}

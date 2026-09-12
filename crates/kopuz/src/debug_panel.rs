//! The debug-only "Database (debug)" settings panel.
//!
//! Lives in the app because the app is what hosts the daemon core, so it is
//! the only place that still holds a write-capable `db::Db`. The settings page
//! renders it through a slot in context, which keeps `pages` and `hooks` free
//! of the database entirely. Compiled to an empty element off the native debug
//! build.

use dioxus::prelude::*;

#[cfg(all(debug_assertions, not(target_os = "android")))]
pub fn debug_db_section() -> Element {
    let db = crate::backend::core()
        .map(|core| core.db.clone())
        .expect("the core is started before the UI mounts");
    let gens = hooks::db_reactivity::use_generations();
    let mut status = use_signal(String::new);

    let bump_all = move || {
        use hooks::db_reactivity::Table;
        for t in [
            Table::Tracks,
            Table::Albums,
            Table::Playlists,
            Table::Favorites,
            Table::Folders,
            Table::Servers,
        ] {
            gens.bump(t);
        }
    };

    let db_reset = db.clone();
    let db_release = db.clone();
    let db_seed = db.clone();
    let db_import = db.clone();
    let db_vacuum = db.clone();
    let db_info = db.clone();

    rsx! {
        section {
            h2 {
                class: "text-lg font-semibold text-white/80 mb-4 border-b border-white/5 pb-2",
                "Database (debug)"
            }
            p { class: "text-xs text-white/40 mb-3",
                "Operates on the debug DB ({db::default_db_path().display()}) — never your release data."
            }
            div { class: "flex flex-wrap gap-3",
                button {
                    class: "px-4 py-2 rounded-lg bg-red-500/20 hover:bg-red-500/30 text-red-300 text-sm transition-colors",
                    onclick: move |_| {
                        let db = db_reset.clone();
                        spawn(async move {
                            match db.debug_reset(&db::default_db_path()).await {
                                Ok(()) => status.set("DB reset to empty schema".into()),
                                Err(e) => status.set(format!("reset failed: {e}")),
                            }
                            bump_all();
                        });
                    },
                    "Reset DB"
                }
                button {
                    class: "px-4 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors",
                    onclick: move |_| {
                        let db = db_release.clone();
                        spawn(async move {
                            match db.debug_load_release(&db::release_db_path(), &db::default_db_path()).await {
                                Ok(()) => status.set("release DB copied in + migrated".into()),
                                Err(e) => status.set(format!("load release failed: {e}")),
                            }
                            bump_all();
                        });
                    },
                    "Load release DB"
                }
                button {
                    class: "px-4 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors",
                    onclick: move |_| {
                        let db = db_import.clone();
                        spawn(async move {
                            match db.import_legacy_json(&db::config_dir()).await {
                                Ok(r) if r.ran => status.set(format!(
                                    "imported: {} tracks, {} albums, {} playlists, {} favorites",
                                    r.tracks, r.albums, r.playlists, r.favorites
                                )),
                                Ok(_) => status.set("import skipped (DB not empty)".into()),
                                Err(e) => status.set(format!("import failed: {e}")),
                            }
                            bump_all();
                        });
                    },
                    "Re-run JSON import"
                }
                button {
                    class: "px-4 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors",
                    onclick: move |_| {
                        let db = db_seed.clone();
                        spawn(async move {
                            match db.debug_seed_synthetic(20_000).await {
                                Ok(()) => status.set("seeded 20k synthetic tracks".into()),
                                Err(e) => status.set(format!("seed failed: {e}")),
                            }
                            bump_all();
                        });
                    },
                    "Seed 20k tracks"
                }
                button {
                    class: "px-4 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors",
                    onclick: move |_| {
                        let db = db_vacuum.clone();
                        spawn(async move {
                            match db.debug_vacuum().await {
                                Ok(()) => status.set("VACUUM done".into()),
                                Err(e) => status.set(format!("vacuum failed: {e}")),
                            }
                        });
                    },
                    "Vacuum"
                }
                button {
                    class: "px-4 py-2 rounded-lg bg-white/10 hover:bg-white/20 text-white text-sm transition-colors",
                    onclick: move |_| {
                        let db = db_info.clone();
                        spawn(async move {
                            match db.debug_info().await {
                                Ok(info) => status.set(info),
                                Err(e) => status.set(format!("info failed: {e}")),
                            }
                        });
                    },
                    "Schema info"
                }
            }
            if !status.read().is_empty() {
                pre { class: "text-xs text-white/60 mt-3 whitespace-pre-wrap", "{status}" }
            }
        }
    }
}

#[cfg(not(all(debug_assertions, not(target_os = "android"))))]
pub fn debug_db_section() -> Element {
    rsx! {}
}

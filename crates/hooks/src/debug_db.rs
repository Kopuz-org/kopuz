//! The debug-only "Database (debug)" settings panel. Lives here (the query
//! layer, which already depends on `db`) rather than in `pages`, so `pages` can
//! drop its `db` dependency entirely and lose the ability to name the
//! write-capable `db::Db`. Compiled to an empty element off the native debug
//! build.

use dioxus::prelude::*;

#[cfg(all(debug_assertions, not(target_os = "android")))]
pub fn debug_db_section() -> Element {
    let db = use_context::<db::Db>();
    let gens = crate::db_reactivity::use_generations();
    let mut status = use_signal(String::new);

    let bump_all = move || {
        use crate::db_reactivity::Table;
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

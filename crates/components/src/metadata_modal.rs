use dioxus::prelude::*;
use reader::models::{CoverChange, Track, TrackEdits};

#[derive(PartialEq, Clone, Props)]
pub struct MetadataModalProps {
    pub track: Track,
    pub on_close: EventHandler,
    /// Persist edited tags. The page handler writes them to the file and
    /// updates the library. Optional — when absent the modal is view-only.
    pub on_save: Option<EventHandler<TrackEdits>>,
}

fn fmt_dur(s: u64) -> String {
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(not(target_arch = "wasm32"))]
fn data_url(bytes: &[u8], mime: &str) -> String {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{b64}")
}

#[component]
pub fn MetadataModal(props: MetadataModalProps) -> Element {
    let t = &props.track;
    let editable = props.on_save.is_some();
    let mut editing = use_signal(|| false);

    let mut title = use_signal(|| props.track.title.clone());
    let mut artist = use_signal(|| {
        if props.track.artists.is_empty() {
            props.track.artist.clone()
        } else {
            props.track.artists.join(", ")
        }
    });
    let mut album = use_signal(|| props.track.album.clone());
    let mut track_no = use_signal(|| {
        props.track.track_number.map(|n| n.to_string()).unwrap_or_default()
    });
    let mut disc_no = use_signal(|| {
        props.track.disc_number.map(|n| n.to_string()).unwrap_or_default()
    });

    let mut cover_preview = use_signal(|| None::<String>);
    let mut cover_change = use_signal(|| CoverChange::Keep);

    {
        let path = props.track.path.clone();
        use_hook(move || {
            #[cfg(not(target_arch = "wasm32"))]
            spawn(async move {
                if let Some((bytes, mime)) = reader::read_cover(&path) {
                    cover_preview.set(Some(data_url(&bytes, &mime)));
                }
            });
            #[cfg(target_arch = "wasm32")]
            let _ = path;
        });
    }

    let mut readonly: Vec<(String, String)> = Vec::new();
    let mut push = |label: &str, value: String| {
        if !value.trim().is_empty() {
            readonly.push((label.to_string(), value));
        }
    };
    if t.duration > 0 {
        push("Duration", fmt_dur(t.duration));
    }
    if t.khz > 0 {
        push("Sample rate", format!("{:.1} kHz", t.khz as f64 / 1000.0));
    }
    if t.bitrate > 0 {
        push("Bitrate", format!("{} kbps", t.bitrate));
    }
    push("MusicBrainz release", t.musicbrainz_release_id.clone().unwrap_or_default());
    push("MusicBrainz recording", t.musicbrainz_recording_id.clone().unwrap_or_default());
    push("MusicBrainz track", t.musicbrainz_track_id.clone().unwrap_or_default());
    push("Path", t.path.display().to_string());

    let input_class = "w-full bg-white/5 border border-white/10 rounded px-3 py-2 text-white text-sm focus:outline-none focus:border-white/20";

    let mut do_save = move || {
        if let Some(handler) = props.on_save {
            let edits = TrackEdits {
                title: title.read().clone(),
                artist: artist.read().clone(),
                album: album.read().clone(),
                track_number: track_no.read().trim().parse::<u32>().ok(),
                disc_number: disc_no.read().trim().parse::<u32>().ok(),
                cover: cover_change.read().clone(),
            };
            handler.call(edits);
        }
        editing.set(false);
    };

    let pick_cover = move |_| {
        #[cfg(all(not(target_arch = "wasm32"), not(target_os = "android")))]
        spawn(async move {
            let file = rfd::AsyncFileDialog::new()
                .add_filter("Images", &["jpg", "jpeg", "png", "webp", "gif"])
                .pick_file()
                .await;
            if let Some(file) = file {
                let mime = match file
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
                    .as_deref()
                {
                    Some("png") => "image/png",
                    Some("gif") => "image/gif",
                    Some("webp") => "image/webp",
                    _ => "image/jpeg",
                };
                let bytes = file.read().await;
                cover_preview.set(Some(data_url(&bytes, mime)));
                cover_change.set(CoverChange::Set(bytes));
            }
        });
    };

    rsx! {
        div {
            class: "fixed inset-0 bg-black/80 flex items-center justify-center z-50",
            onclick: move |_| props.on_close.call(()),
            div {
                class: "bg-neutral-900 rounded-xl border border-white/10 w-full max-w-lg p-6",
                onclick: move |e| e.stop_propagation(),

                div { class: "flex items-center justify-between mb-4",
                    h2 { class: "text-xl font-bold text-white",
                        if *editing.read() { "Edit metadata" } else { "Metadata" }
                    }
                    div { class: "flex items-center gap-1",
                        if editable && !*editing.read() {
                            button {
                                class: "w-8 h-8 flex items-center justify-center rounded-full text-slate-400 hover:text-white hover:bg-white/10 transition-colors",
                                title: "Edit",
                                onclick: move |_| editing.set(true),
                                i { class: "fa-solid fa-pen" }
                            }
                        }
                        button {
                            class: "w-8 h-8 flex items-center justify-center rounded-full text-slate-400 hover:text-white hover:bg-white/10 transition-colors",
                            onclick: move |_| props.on_close.call(()),
                            i { class: "fa-solid fa-xmark" }
                        }
                    }
                }

                div { class: "flex items-center gap-4 mb-4",
                    div {
                        class: "w-24 h-24 rounded-lg overflow-hidden shrink-0 bg-white/5 flex items-center justify-center",
                        if let Some(url) = cover_preview.read().clone() {
                            img { src: "{url}", class: "w-full h-full object-cover" }
                        } else {
                            i { class: "fa-solid fa-music text-white/20 text-2xl" }
                        }
                    }
                    if *editing.read() {
                        div { class: "flex flex-col gap-2",
                            button {
                                class: "bg-white/10 hover:bg-white/20 text-white px-3 py-2 rounded text-sm transition-colors flex items-center gap-2",
                                onclick: pick_cover,
                                i { class: "fa-solid fa-image" }
                                if cover_preview.read().is_some() { "Change photo" } else { "Add photo" }
                            }
                            if cover_preview.read().is_some() {
                                button {
                                    class: "text-red-400 hover:text-red-300 px-3 py-2 rounded text-sm transition-colors flex items-center gap-2",
                                    onclick: move |_| {
                                        cover_preview.set(None);
                                        cover_change.set(CoverChange::Remove);
                                    },
                                    i { class: "fa-solid fa-trash" }
                                    "Remove photo"
                                }
                            }
                        }
                    }
                }

                div { class: "max-h-[60vh] overflow-y-auto space-y-3",
                    if *editing.read() {
                        div { class: "flex flex-col gap-1",
                            span { class: "text-[10px] font-bold tracking-widest uppercase text-white/35", "Title" }
                            input {
                                class: input_class,
                                value: "{title}",
                                oninput: move |e| title.set(e.value()),
                                onkeydown: move |e| e.stop_propagation(),
                            }
                        }
                        div { class: "flex flex-col gap-1",
                            span { class: "text-[10px] font-bold tracking-widest uppercase text-white/35", "Artist" }
                            input {
                                class: input_class,
                                value: "{artist}",
                                oninput: move |e| artist.set(e.value()),
                                onkeydown: move |e| e.stop_propagation(),
                            }
                        }
                        div { class: "flex flex-col gap-1",
                            span { class: "text-[10px] font-bold tracking-widest uppercase text-white/35", "Album" }
                            input {
                                class: input_class,
                                value: "{album}",
                                oninput: move |e| album.set(e.value()),
                                onkeydown: move |e| e.stop_propagation(),
                            }
                        }
                        div { class: "flex gap-3",
                            div { class: "flex flex-col gap-1 flex-1",
                                span { class: "text-[10px] font-bold tracking-widest uppercase text-white/35", "Track #" }
                                input {
                                    r#type: "number",
                                    class: input_class,
                                    value: "{track_no}",
                                    oninput: move |e| track_no.set(e.value()),
                                    onkeydown: move |e| e.stop_propagation(),
                                }
                            }
                            div { class: "flex flex-col gap-1 flex-1",
                                span { class: "text-[10px] font-bold tracking-widest uppercase text-white/35", "Disc #" }
                                input {
                                    r#type: "number",
                                    class: input_class,
                                    value: "{disc_no}",
                                    oninput: move |e| disc_no.set(e.value()),
                                    onkeydown: move |e| e.stop_propagation(),
                                }
                            }
                        }
                        p { class: "text-xs text-white/30 italic",
                            "Empty fields remove that tag. Writes directly to the file — no undo."
                        }
                    } else {
                        MetaRow { label: "Title".to_string(), value: title.read().clone() }
                        MetaRow { label: "Artist".to_string(), value: artist.read().clone() }
                        MetaRow { label: "Album".to_string(), value: album.read().clone() }
                        if !track_no.read().trim().is_empty() {
                            MetaRow { label: "Track #".to_string(), value: track_no.read().clone() }
                        }
                        if !disc_no.read().trim().is_empty() {
                            MetaRow { label: "Disc #".to_string(), value: disc_no.read().clone() }
                        }
                    }

                    for (label, value) in readonly {
                        MetaRow { key: "{label}", label, value }
                    }
                }

                if *editing.read() {
                    div { class: "mt-6 flex justify-end gap-2",
                        button {
                            class: "text-slate-400 hover:text-white text-sm transition-colors px-3 py-2",
                            onclick: move |_| editing.set(false),
                            "Cancel"
                        }
                        button {
                            class: "bg-white text-black px-4 py-2 rounded text-sm font-medium hover:bg-slate-200 transition-colors",
                            onclick: move |_| do_save(),
                            "Save"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn MetaRow(label: String, value: String) -> Element {
    if value.trim().is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "flex flex-col gap-0.5",
            span { class: "text-[10px] font-bold tracking-widest uppercase text-white/35", "{label}" }
            span { class: "text-sm text-white break-all select-text", "{value}" }
        }
    }
}

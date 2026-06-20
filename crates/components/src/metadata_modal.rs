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
        props
            .track
            .track_number
            .map(|n| n.to_string())
            .unwrap_or_default()
    });
    let mut disc_no = use_signal(|| {
        props
            .track
            .disc_number
            .map(|n| n.to_string())
            .unwrap_or_default()
    });

    let mut cover_preview = use_signal(|| None::<String>);
    let mut cover_change = use_signal(|| CoverChange::Keep);

    {
        let path = props.track.id.local_path().map(|p| p.to_path_buf());
        use_hook(move || {
            #[cfg(not(target_arch = "wasm32"))]
            spawn(async move {
                if let Some(p) = &path
                    && let Some((bytes, mime)) = reader::read_cover(p)
                {
                    cover_preview.set(Some(data_url(&bytes, &mime)));
                }
            });
            #[cfg(target_arch = "wasm32")]
            let _ = path;
        });
    }

    let metadata_text = i18n::t("metadata").to_string();
    let edit_metadata_text = i18n::t("edit_metadata").to_string();
    let edit_text = i18n::t("edit").to_string();
    let title_text = i18n::t("title").to_string();
    let artist_text = i18n::t("artist").to_string();
    let album_text = i18n::t("album").to_string();
    let track_number_text = i18n::t("track_number").to_string();
    let disc_number_text = i18n::t("disc_number").to_string();
    let duration_text = i18n::t("duration").to_string();
    let sample_rate_text = i18n::t("sample_rate").to_string();
    let bitrate_text = i18n::t("bitrate").to_string();
    let musicbrainz_release_text = i18n::t("musicbrainz_release").to_string();
    let musicbrainz_recording_text = i18n::t("musicbrainz_recording").to_string();
    let musicbrainz_track_text = i18n::t("musicbrainz_track").to_string();
    let path_text = i18n::t("path").to_string();
    let add_photo_text = i18n::t("add_photo").to_string();
    let change_photo_text = i18n::t("change_photo").to_string();
    let remove_photo_text = i18n::t("remove_photo").to_string();
    let metadata_edit_warning_text = i18n::t("metadata_edit_warning").to_string();
    let cancel_text = i18n::t("cancel").to_string();
    let save_text = i18n::t("save").to_string();

    let mut readonly: Vec<(String, String)> = Vec::new();
    let mut push = |label: &str, value: String| {
        if !value.trim().is_empty() {
            readonly.push((label.to_string(), value));
        }
    };
    if t.duration > 0 {
        push(&duration_text, fmt_dur(t.duration));
    }
    if t.khz > 0 {
        push(
            &sample_rate_text,
            format!("{:.1} kHz", t.khz as f64 / 1000.0),
        );
    }
    if t.bitrate > 0 {
        push(&bitrate_text, format!("{} kbps", t.bitrate));
    }
    push(
        &musicbrainz_release_text,
        t.musicbrainz_release_id.clone().unwrap_or_default(),
    );
    push(
        &musicbrainz_recording_text,
        t.musicbrainz_recording_id.clone().unwrap_or_default(),
    );
    push(
        &musicbrainz_track_text,
        t.musicbrainz_track_id.clone().unwrap_or_default(),
    );
    push(&path_text, t.id.uid());

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
                        if *editing.read() { "{edit_metadata_text}" } else { "{metadata_text}" }
                    }
                    div { class: "flex items-center gap-1",
                        if editable && !*editing.read() {
                            button {
                                class: "w-8 h-8 flex items-center justify-center rounded-full text-slate-400 hover:text-white hover:bg-white/10 transition-colors",
                                title: "{edit_text}",
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
                                if cover_preview.read().is_some() { "{change_photo_text}" } else { "{add_photo_text}" }
                            }
                            if cover_preview.read().is_some() {
                                button {
                                    class: "text-red-400 hover:text-red-300 px-3 py-2 rounded text-sm transition-colors flex items-center gap-2",
                                    onclick: move |_| {
                                        cover_preview.set(None);
                                        cover_change.set(CoverChange::Remove);
                                    },
                                    i { class: "fa-solid fa-trash" }
                                    "{remove_photo_text}"
                                }
                            }
                        }
                    }
                }

                div { class: "max-h-[60vh] overflow-y-auto space-y-3",
                    if *editing.read() {
                        div { class: "flex flex-col gap-1",
                            span { class: "text-[10px] font-bold tracking-widest uppercase text-white/35", "{title_text}" }
                            input {
                                class: input_class,
                                value: "{title}",
                                oninput: move |e| title.set(e.value()),
                                onkeydown: move |e| e.stop_propagation(),
                            }
                        }
                        div { class: "flex flex-col gap-1",
                            span { class: "text-[10px] font-bold tracking-widest uppercase text-white/35", "{artist_text}" }
                            input {
                                class: input_class,
                                value: "{artist}",
                                oninput: move |e| artist.set(e.value()),
                                onkeydown: move |e| e.stop_propagation(),
                            }
                        }
                        div { class: "flex flex-col gap-1",
                            span { class: "text-[10px] font-bold tracking-widest uppercase text-white/35", "{album_text}" }
                            input {
                                class: input_class,
                                value: "{album}",
                                oninput: move |e| album.set(e.value()),
                                onkeydown: move |e| e.stop_propagation(),
                            }
                        }
                        div { class: "flex gap-3",
                            div { class: "flex flex-col gap-1 flex-1",
                                span { class: "text-[10px] font-bold tracking-widest uppercase text-white/35", "{track_number_text}" }
                                input {
                                    r#type: "number",
                                    class: input_class,
                                    value: "{track_no}",
                                    oninput: move |e| track_no.set(e.value()),
                                    onkeydown: move |e| e.stop_propagation(),
                                }
                            }
                            div { class: "flex flex-col gap-1 flex-1",
                                span { class: "text-[10px] font-bold tracking-widest uppercase text-white/35", "{disc_number_text}" }
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
                            "{metadata_edit_warning_text}"
                        }
                    } else {
                        MetaRow { label: title_text.clone(), value: title.read().clone() }
                        MetaRow { label: artist_text.clone(), value: artist.read().clone() }
                        MetaRow { label: album_text.clone(), value: album.read().clone() }
                        if !track_no.read().trim().is_empty() {
                            MetaRow { label: track_number_text.clone(), value: track_no.read().clone() }
                        }
                        if !disc_no.read().trim().is_empty() {
                            MetaRow { label: disc_number_text.clone(), value: disc_no.read().clone() }
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
                            "{cancel_text}"
                        }
                        button {
                            class: "bg-white text-black px-4 py-2 rounded text-sm font-medium hover:bg-slate-200 transition-colors",
                            onclick: move |_| do_save(),
                            "{save_text}"
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

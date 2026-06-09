use dioxus::prelude::*;
use reader::models::Track;

#[derive(PartialEq, Clone, Props)]
pub struct MetadataModalProps {
    pub track: Track,
    pub on_close: EventHandler,
}

fn fmt_dur(s: u64) -> String {
    format!("{}:{:02}", s / 60, s % 60)
}

#[component]
pub fn MetadataModal(props: MetadataModalProps) -> Element {
    let t = &props.track;

    // (label, value) rows. Skip rows whose value is empty / unknown.
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut push = |label: &str, value: String| {
        if !value.trim().is_empty() {
            rows.push((label.to_string(), value));
        }
    };

    push("Title", t.title.clone());
    let artists = if t.artists.is_empty() {
        t.artist.clone()
    } else {
        t.artists.join(", ")
    };
    push("Artist", artists);
    push("Album", t.album.clone());
    if let Some(n) = t.track_number {
        push("Track #", n.to_string());
    }
    if let Some(n) = t.disc_number {
        push("Disc #", n.to_string());
    }
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

    rsx! {
        div {
            class: "fixed inset-0 bg-black/80 flex items-center justify-center z-50",
            onclick: move |_| props.on_close.call(()),
            div {
                class: "bg-neutral-900 rounded-xl border border-white/10 w-full max-w-lg p-6",
                onclick: move |e| e.stop_propagation(),

                div { class: "flex items-center justify-between mb-4",
                    h2 { class: "text-xl font-bold text-white", "Metadata" }
                    button {
                        class: "w-8 h-8 flex items-center justify-center rounded-full text-slate-400 hover:text-white hover:bg-white/10 transition-colors",
                        onclick: move |_| props.on_close.call(()),
                        i { class: "fa-solid fa-xmark" }
                    }
                }

                div { class: "max-h-[60vh] overflow-y-auto space-y-3",
                    for (label, value) in rows {
                        div {
                            key: "{label}",
                            class: "flex flex-col gap-0.5",
                            span { class: "text-[10px] font-bold tracking-widest uppercase text-white/35", "{label}" }
                            span { class: "text-sm text-white break-all select-text", "{value}" }
                        }
                    }
                }
            }
        }
    }
}

use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

pub(crate) fn use_fullscreen_lyrics(
    current_song_title: Signal<String>,
    current_song_artist: Signal<String>,
    current_song_album: Signal<String>,
    current_song_duration: Signal<u64>,
) -> Signal<Option<Option<utils::lyrics::Lyrics>>> {
    let ctrl = use_context::<PlayerController>();
    let api = use_context::<std::sync::Arc<dyn api::KopuzApi>>();
    let mut lyrics: Signal<Option<Option<utils::lyrics::Lyrics>>> = use_signal(|| None);
    let mut fetch_gen: Signal<u32> = use_signal(|| 0);
    let mut last_key: Signal<String> = use_signal(String::new);
    let mut fetch_task = use_signal(|| None::<dioxus_core::Task>);

    use_effect(move || {
        let current_track = ctrl.current_track_snapshot.read().clone();
        let key = current_track
            .as_ref()
            .map(|track| track.id.key().into_owned())
            .unwrap_or_default();
        let title = current_track
            .as_ref()
            .map(|track| track.title.clone())
            .unwrap_or_else(|| current_song_title.read().clone());
        let fallback_artist = current_song_artist.read().clone();
        let fallback_album = current_song_album.read().clone();
        let fallback_duration = *current_song_duration.read();
        let new_key =
            format!("{title}|{key}|{fallback_artist}|{fallback_album}|{fallback_duration}");
        if *last_key.peek() == new_key {
            return;
        }
        last_key.set(new_key);
        if let Some(task) = fetch_task.take() {
            task.cancel();
        }
        let fetch_id = fetch_gen.peek().wrapping_add(1);
        fetch_gen.set(fetch_id);
        if title.is_empty()
            || current_track
                .as_ref()
                .is_some_and(|track| track.duration == u64::MAX)
            || key.is_empty()
        {
            lyrics.set(Some(None));
            return;
        }
        lyrics.set(None);
        let api = api.clone();
        let task = spawn(async move {
            use futures_util::StreamExt as _;
            let mut last_displayed: Option<utils::lyrics::Lyrics> = None;
            let mut stream = api.lyrics_stream(key);
            while let Some(result) = stream.next().await {
                if *fetch_gen.peek() != fetch_id {
                    return;
                }
                match result {
                    Ok(value) => {
                        let display = crate::lyrics_view::from_api(value);
                        if last_displayed.as_ref() != Some(&display) {
                            last_displayed = Some(display.clone());
                            lyrics.set(Some(Some(display)));
                        }
                    }
                    Err(_) if last_displayed.is_none() => {
                        lyrics.set(Some(Some(utils::lyrics::Lyrics::Plain(
                            i18n::t("lyrics_not_found").to_string(),
                        ))));
                    }
                    Err(_) => {}
                }
            }
        });
        fetch_task.set(Some(task));
    });

    lyrics
}

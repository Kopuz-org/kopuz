use config::AppConfig;
use dioxus::prelude::*;
use hooks::use_player_controller::PlayerController;

pub(crate) fn use_fullscreen_lyrics(
    current_song_title: Signal<String>,
    current_song_artist: Signal<String>,
    current_song_album: Signal<String>,
    current_song_duration: Signal<u64>,
) -> Signal<Option<Option<utils::lyrics::Lyrics>>> {
    let ctrl = use_context::<PlayerController>();
    let config = use_context::<Signal<AppConfig>>();
    let mut lyrics: Signal<Option<Option<utils::lyrics::Lyrics>>> = use_signal(|| None);
    let mut fetch_gen: Signal<u32> = use_signal(|| 0);
    let mut last_key: Signal<String> = use_signal(String::new);

    use_effect(move || {
        let current_track = ctrl.current_track_snapshot.read().clone();

        let (title, artist, album, duration, track_path) = if let Some(track) = current_track {
            (
                track.title,
                track.artist,
                track.album,
                track.duration,
                track.id.uid(),
            )
        } else {
            (
                current_song_title.read().clone(),
                current_song_artist.read().clone(),
                current_song_album.read().clone(),
                *current_song_duration.read(),
                String::new(),
            )
        };

        let new_key = format!("{title}|{track_path}");
        if *last_key.peek() == new_key {
            return;
        }
        last_key.set(new_key);
        let (server_url, server_token, server_user_id, prefer_local, enable_musixmatch) = {
            let conf = config.peek();
            let prefer_local = conf.prefer_local_lyrics;
            let enable_musixmatch = conf.enable_musixmatch_lyrics;
            if let Some(server) = &conf.server {
                (
                    Some(server.url.clone()),
                    server.access_token.clone(),
                    server.user_id.clone(),
                    prefer_local,
                    enable_musixmatch,
                )
            } else {
                (None, None, None, prefer_local, enable_musixmatch)
            }
        };

        let fetch_id = fetch_gen.peek().wrapping_add(1);
        fetch_gen.set(fetch_id);

        // Radio has no lyrics; querying providers with station names only
        // produces junk matches.
        if title.is_empty() || hooks::playback_ref::PlaybackItemRef::parse(&track_path).is_radio() {
            lyrics.set(Some(None));
            return;
        }

        let track_path_for_spawn = track_path.clone();
        let lyrics_request =
            utils::lyrics::LyricsRequest::new(artist, title, album, duration, track_path)
                .with_server(
                    server_url.as_deref(),
                    server_token.as_deref(),
                    server_user_id.as_deref(),
                )
                .prefer_local(prefer_local)
                .enable_musixmatch(enable_musixmatch);

        if let Some(cached) = utils::lyrics::cached_lyrics_for_request(&lyrics_request) {
            let display = cached.or_else(|| {
                Some(utils::lyrics::Lyrics::Plain(
                    i18n::t("lyrics_not_found").to_string(),
                ))
            });
            lyrics.set(Some(display));
            return;
        }

        lyrics.set(None);

        spawn(async move {
            // Lazily attach Apple Music auth for the lyrics provider.
            let lyrics_request = if track_path_for_spawn.starts_with("applemusic:") {
                let am_auth = config.peek().server.as_ref().and_then(|server| {
                    if server.service != config::MusicService::AppleMusic {
                        return None;
                    }
                    let token = server.access_token.clone()?;
                    let catalog_id = track_path_for_spawn
                        .strip_prefix("applemusic:")
                        .unwrap_or(&track_path_for_spawn)
                        .to_string();
                    Some(utils::lyrics::AppleMusicLyricsAuth {
                        token,
                        bearer_token: String::new(),
                        storefront: server.apple_music_storefront.clone(),
                        language: server.apple_music_language.clone(),
                        catalog_id,
                    })
                });
                if let Some(mut auth) = am_auth {
                    if let Ok(bt) = ::server::applemusic::auth::get_bearer_token().await {
                        auth.bearer_token = bt;
                    }
                    lyrics_request.apple_music_auth(auth)
                } else {
                    lyrics_request
                }
            } else {
                lyrics_request
            };
            let mut last_displayed: Option<utils::lyrics::Lyrics> = None;
            let result =
                utils::lyrics::fetch_lyrics_progressive_for_request(&lyrics_request, |partial| {
                    if *fetch_gen.peek() == fetch_id && last_displayed.as_ref() != Some(&partial) {
                        last_displayed = Some(partial.clone());
                        lyrics.set(Some(Some(partial)));
                    }
                })
                .await;
            if *fetch_gen.peek() == fetch_id {
                let display = result.or_else(|| {
                    Some(utils::lyrics::Lyrics::Plain(
                        i18n::t("lyrics_not_found").to_string(),
                    ))
                });
                if display.as_ref() != last_displayed.as_ref() {
                    lyrics.set(Some(display));
                }
            }
        });
    });

    lyrics
}

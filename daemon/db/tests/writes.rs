//! Batch upsert + scan-reconcile prune (issue #347, step 7).

use std::path::PathBuf;

use db::{Page, Source, TrackFilter};
use reader::models::{Album, Track, TrackId};

fn unique_db() -> PathBuf {
    // pid + counter, not just clock: macOS's µs clock let parallel tests
    // collide on a nanos-only name and delete each other's live DB.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("kopuz-w-{pid}-{nanos}-{seq}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

fn local(path: &str, title: &str) -> Track {
    Track {
        id: TrackId::Local(PathBuf::from(path)),
        cover: None,
        album_id: "alb".into(),
        title: title.into(),
        artist: "Artist".into(),
        album: "Album".into(),
        duration: 123,
        khz: 44100,
        bitrate: 900,
        track_number: Some(2),
        disc_number: Some(1),
        musicbrainz_release_id: Some("mbr".into()),
        musicbrainz_recording_id: None,
        musicbrainz_track_id: None,
        playlist_item_id: None,
        artists: vec!["Artist".into(), "Feat".into()],
        replay_gain: config::ReplayGainInfo::default(),
        counterpart: None,
        credits: Vec::new(),
        explicit: false,
        plays: None,
    }
}

#[tokio::test]
async fn upsert_then_prune() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    let a = local("/music/a.flac", "A");
    let b = local("/music/b.flac", "B");
    let c = local("/other/c.flac", "C");
    db.upsert_tracks(&Source::default(), &[a.clone(), b.clone(), c.clone()])
        .await
        .unwrap();

    let filter = TrackFilter::new(Source::default());
    assert_eq!(db.tracks_count(&filter).await.unwrap(), 3);

    // Upsert is idempotent on identity: re-inserting "A" with a new title updates
    // the existing row rather than adding one.
    let mut a2 = a.clone();
    a2.title = "A (remastered)".into();
    db.upsert_tracks(&Source::default(), &[a2]).await.unwrap();
    assert_eq!(db.tracks_count(&filter).await.unwrap(), 3);

    // Round-trip preserves the typed fields.
    let page = db
        .tracks_page(
            &filter,
            Page {
                offset: 0,
                limit: 10,
            },
        )
        .await
        .unwrap();
    let got = page.iter().find(|t| t.title.starts_with("A")).unwrap();
    assert_eq!(got.title, "A (remastered)");
    assert_eq!(got.track_number, Some(2));
    assert_eq!(got.musicbrainz_release_id.as_deref(), Some("mbr"));
    assert_eq!(got.artists, vec!["Artist".to_string(), "Feat".to_string()]);
    assert!(matches!(got.id, TrackId::Local(_)));

    // Prune the local source keeping "a.flac" + "c.flac" → "b.flac" goes (the
    // scan-reconcile step: anything not in the last scan's keep-set).
    let keep = vec!["/music/a.flac".to_string(), "/other/c.flac".to_string()];
    db.prune_source(&Source::default(), &keep, &[])
        .await
        .unwrap();
    assert_eq!(db.tracks_count(&filter).await.unwrap(), 2);
    let remaining: Vec<String> = db
        .tracks_page(
            &filter,
            Page {
                offset: 0,
                limit: 10,
            },
        )
        .await
        .unwrap()
        .iter()
        .filter_map(|t| t.id.local_path().map(|p| p.to_string_lossy().into_owned()))
        .collect();
    assert!(remaining.contains(&"/music/a.flac".to_string()));
    assert!(remaining.contains(&"/other/c.flac".to_string()));
    assert!(!remaining.contains(&"/music/b.flac".to_string()));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn automatic_cover_update_preserves_concurrent_manual_cover() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    let album = Album {
        id: "album".into(),
        title: "Album".into(),
        artist: "Artist".into(),
        genre: "Unknown".into(),
        year: 0,
        cover_path: None,
        manual_cover: false,
        artist_id: None,
        artist_key: None,
    };
    db.upsert_albums(&Source::default(), &[album])
        .await
        .unwrap();

    assert!(
        db.update_album_cover_if_not_manual(&Source::default(), "album", "/auto.jpg")
            .await
            .unwrap()
    );
    db.update_album_cover(&Source::default(), "album", Some("/manual.jpg"), true)
        .await
        .unwrap();
    assert!(
        !db.update_album_cover_if_not_manual(&Source::default(), "album", "/late-auto.jpg")
            .await
            .unwrap()
    );

    let stored = db
        .album(&Source::default(), "album")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.cover_path, Some(PathBuf::from("/manual.jpg")));
    assert!(stored.manual_cover);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn a_rebilled_album_takes_the_new_artist_whole() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    let source = Source::Server("srv".into());
    let album = |artist: &str, artist_id: Option<&str>| Album {
        id: "MPRE1".into(),
        title: "Album".into(),
        artist: artist.into(),
        genre: String::new(),
        year: 0,
        cover_path: None,
        manual_cover: false,
        artist_id: artist_id.map(Into::into),
        artist_key: artist_id.map(Into::into),
    };

    db.upsert_albums(&source, &[album("Ada", Some("UC-a"))])
        .await
        .unwrap();
    db.upsert_albums(&source, &[album("Boris", None)])
        .await
        .unwrap();

    let stored = db.album(&source, "MPRE1").await.unwrap().unwrap();
    assert_eq!(stored.artist, "Boris");
    assert_eq!(
        stored.artist_id, None,
        "Ada's id would open the wrong artist"
    );

    db.upsert_albums(&source, &[album("", None)]).await.unwrap();
    let stored = db.album(&source, "MPRE1").await.unwrap().unwrap();
    assert_eq!(stored.artist_key, None, "billed to nobody");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

/// First coverage of the metadata-cache API: `meta_keys_since` returns keys of
/// the requested kind written within the window, and a re-put refreshes the
/// `fetched_at` stamp (the artist-photo-miss TTL relies on both).
#[tokio::test]
async fn meta_keys_since_windows_by_kind_and_age() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    db.meta_put("artist a", "artist_photo_miss", "")
        .await
        .unwrap();
    db.meta_put("artist b", "other_kind", "").await.unwrap();

    let fresh = db
        .meta_keys_since("artist_photo_miss", 86_400)
        .await
        .unwrap();
    assert_eq!(fresh, vec!["artist a".to_string()], "same kind, in window");

    // `fetched_at` can't be backdated through the public API, so expiry is
    // simulated with a negative window: `fetched_at >= unixepoch() + 1` never
    // matches a just-written row.
    let expired = db.meta_keys_since("artist_photo_miss", -1).await.unwrap();
    assert!(expired.is_empty(), "an aged-out row stops matching");

    // Re-putting refreshes the stamp — the row is fresh again by upsert.
    db.meta_put("artist a", "artist_photo_miss", "")
        .await
        .unwrap();
    let fresh = db.meta_keys_since("artist_photo_miss", 1).await.unwrap();
    assert_eq!(fresh, vec!["artist a".to_string()]);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

/// Tracks reach the store from several places, and only some of them know which
/// album a track belongs to — a server's album relationship has to be requested
/// explicitly, so a response without it means "not asked for", not "no album".
///
/// Before this guard, opening a playlist re-wrote its tracks with a blank album
/// id and unlinked them from the album the library sync had just matched, which
/// showed up as albums that render with no songs in them.
#[tokio::test]
async fn a_blank_album_id_does_not_erase_a_known_one() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    let linked = local("/music/a.flac", "A");
    assert_eq!(linked.album_id, "alb");
    db.upsert_tracks(&Source::default(), std::slice::from_ref(&linked))
        .await
        .unwrap();

    // The same track written again by a path that didn't resolve the album.
    let mut album_less = linked.clone();
    album_less.album_id = String::new();
    album_less.title = "A (from a playlist)".into();
    db.upsert_tracks(&Source::default(), &[album_less])
        .await
        .unwrap();

    let tracks = db.album_tracks(&Source::default(), "alb").await.unwrap();
    assert_eq!(
        tracks.len(),
        1,
        "the track must still belong to its album after an album-less write"
    );
    assert_eq!(
        tracks[0].title, "A (from a playlist)",
        "everything the later write did know still lands"
    );

    // A non-empty id is still authoritative — this must not become write-once.
    let mut moved = linked.clone();
    moved.album_id = "alb2".into();
    db.upsert_tracks(&Source::default(), &[moved])
        .await
        .unwrap();
    assert!(
        db.album_tracks(&Source::default(), "alb")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        db.album_tracks(&Source::default(), "alb2")
            .await
            .unwrap()
            .len(),
        1
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn replay_gain_round_trips_and_clears() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    let mut track = local("/music/rg.flac", "RG");
    track.replay_gain = config::ReplayGainInfo {
        track_gain_db: Some(-7.5),
        track_peak: Some(0.98),
        album_gain_db: Some(-5.25),
        album_peak: None,
        loudness_db: None,
    };
    db.upsert_tracks(&Source::default(), std::slice::from_ref(&track))
        .await
        .unwrap();

    let filter = TrackFilter::new(Source::default());
    let stored = db
        .tracks_page(
            &filter,
            Page {
                offset: 0,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(stored[0].replay_gain, track.replay_gain);

    // A re-sync from a server that stopped reporting gain must clear the row,
    // not leave the old values to level a re-encoded file.
    track.replay_gain = config::ReplayGainInfo::default();
    db.upsert_tracks(&Source::default(), std::slice::from_ref(&track))
        .await
        .unwrap();
    let stored = db
        .tracks_page(
            &filter,
            Page {
                offset: 0,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert!(stored[0].replay_gain.is_empty());
    assert_eq!(stored[0].replay_gain.track_peak, None);

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

/// Skip explicit reads the flag off stored rows, not only freshly fetched ones.
#[tokio::test]
async fn explicit_and_plays_round_trip() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    let source = Source::Server("yt".into());
    let mut track = local("/music/e.flac", "E");
    track.id = TrackId::Server {
        service: config::MusicService::YtMusic,
        item_id: "vid".into(),
    };
    track.explicit = true;
    track.plays = Some("900M plays".into());
    db.upsert_tracks(&source, std::slice::from_ref(&track))
        .await
        .unwrap();
    let filter = TrackFilter::new(source.clone());
    let page = Page {
        offset: 0,
        limit: 10,
    };
    let stored = db.tracks_page(&filter, page).await.unwrap();
    assert!(stored[0].explicit);
    assert_eq!(stored[0].plays.as_deref(), Some("900M plays"));

    // A listing that omits the flag or the count (a card, not a track row) leaves the stored ones.
    track.explicit = false;
    track.plays = None;
    db.upsert_tracks(&source, std::slice::from_ref(&track))
        .await
        .unwrap();
    let stored = db.tracks_page(&filter, page).await.unwrap();
    assert!(stored[0].explicit);
    assert_eq!(stored[0].plays.as_deref(), Some("900M plays"));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn a_derived_album_follows_its_track_once_the_credit_is_linked() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    let source = Source::Server("srv".into());
    let track = |credit: reader::ArtistCredit| Track {
        id: TrackId::Server {
            service: config::MusicService::YtMusic,
            item_id: "v1".into(),
        },
        artist: "Ada".into(),
        artists: vec!["Ada".into()],
        credits: vec![credit],
        ..local("/unused", "Song")
    };

    db.upsert_tracks(&source, &[track(reader::ArtistCredit::unlinked("Ada"))])
        .await
        .unwrap();
    db.upsert_tracks(
        &source,
        &[track(reader::ArtistCredit::linked("Ada", "UC-ada"))],
    )
    .await
    .unwrap();

    let album = db.album(&source, "alb").await.unwrap().unwrap();
    let artist = db
        .artist(&source, album.artist_key.as_deref().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(artist.source_id.as_deref(), Some("UC-ada"));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn a_derived_album_is_billed_by_all_its_tracks_not_the_last_one_written() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    let source = Source::Server("srv".into());
    let track = |id: &str, album: &str, artist: &str, credits: Vec<reader::ArtistCredit>| Track {
        id: TrackId::Server {
            service: config::MusicService::YtMusic,
            item_id: id.into(),
        },
        album_id: album.into(),
        artist: artist.into(),
        artists: credits.iter().map(|credit| credit.name.clone()).collect(),
        credits,
        ..local("/unused", "Song")
    };
    let unlinked = reader::ArtistCredit::unlinked;
    let both = || {
        vec![
            reader::ArtistCredit::linked("Kana", "UC-kana"),
            reader::ArtistCredit::linked("Rin", "UC-rin"),
        ]
    };
    db.upsert_tracks(
        &source,
        &[
            track("e1", "emo", "Nat / Teto", vec![unlinked("Nat / Teto")]),
            track(
                "e2",
                "emo",
                "Nat / Teto / Kiri",
                vec![unlinked("Nat / Teto / Kiri")],
            ),
            track(
                "e3",
                "emo",
                "Nat / Gumi / Teto",
                vec![unlinked("Nat / Gumi / Teto")],
            ),
            track("d1", "duo", "Kana & Rin", both()),
            track("d2", "duo", "Kana & Rin", both()),
        ],
    )
    .await
    .unwrap();

    let mut billed = Vec::new();
    for album in ["emo", "duo"] {
        let row = db.album(&source, album).await.unwrap().unwrap();
        let artist = db
            .artist(&source, row.artist_key.as_deref().unwrap())
            .await
            .unwrap()
            .unwrap();
        billed.push((artist.name, artist.source_id));
    }
    assert_eq!(
        billed[0],
        ("Nat / Teto".to_string(), None),
        "no shared credit keeps the byline's own row"
    );
    assert_eq!(billed[1].1.as_deref(), Some("UC-kana"));

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn a_derived_album_byline_never_picks_a_credit_by_its_name() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    let source = Source::Server("srv".into());
    let track = |id: &str, artist: &str, credits: Vec<reader::ArtistCredit>| Track {
        id: TrackId::Server {
            service: config::MusicService::YtMusic,
            item_id: id.into(),
        },
        album_id: "alb".into(),
        artist: artist.into(),
        artists: credits.iter().map(|credit| credit.name.clone()).collect(),
        credits,
        ..local("/unused", "Song")
    };
    db.upsert_tracks(
        &source,
        &[
            track(
                "t1",
                "Rin",
                vec![
                    reader::ArtistCredit::linked("Kana", "UC-kana"),
                    reader::ArtistCredit::linked("Rin", "UC-rin"),
                ],
            ),
            track(
                "t2",
                "Mio",
                vec![reader::ArtistCredit::linked("Mio", "UC-mio")],
            ),
        ],
    )
    .await
    .unwrap();

    let album = db.album(&source, "alb").await.unwrap().unwrap();
    let billed = db
        .artist(&source, album.artist_key.as_deref().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        billed.source_id.as_deref(),
        Some("UC-kana"),
        "the founding track's first credit, though the byline spells another"
    );
    let sampled: Vec<String> = db
        .artist_sample_tracks(&source, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|track| track.id.key().into_owned())
        .collect();
    assert_eq!(
        sampled,
        ["t1", "t2"],
        "tiles sort by billed name: t1 bills Kana, its first credit, ahead of Mio, not Rin after"
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

#[tokio::test]
async fn a_joined_album_artist_is_billed_to_the_artists_its_tracks_credit() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    let source = Source::default();
    let track = |path: &str, album_id: &str, artist: &str, artists: &[&str]| Track {
        album_id: album_id.into(),
        artist: artist.into(),
        artists: artists.iter().map(|name| name.to_string()).collect(),
        ..local(path, "Song")
    };
    let album = |id: &str, artist: &str| Album {
        id: id.into(),
        title: id.into(),
        artist: artist.into(),
        genre: String::new(),
        year: 0,
        cover_path: None,
        manual_cover: false,
        artist_id: None,
        artist_key: None,
    };
    db.upsert_tracks(
        &source,
        &[
            track(
                "/m/k/1.flac",
                "duo",
                "Cartoon, Jéja feat. Coleman Trapp",
                &["Cartoon", "Jéja", "Coleman Trapp"],
            ),
            track(
                "/m/k/2.flac",
                "duo",
                "Cartoon, JÉJA, Futuristik",
                &["Cartoon", "JÉJA", "Futuristik"],
            ),
            track("/m/v/1.flac", "va", "Janji", &["Janji"]),
            track("/m/v/2.flac", "va", "DEAF KEV", &["DEAF KEV"]),
        ],
    )
    .await
    .unwrap();
    db.upsert_albums(
        &source,
        &[
            album("duo", "Cartoon, Jéja"),
            album("va", "Various Artists"),
        ],
    )
    .await
    .unwrap();

    let names: Vec<String> = db
        .artists(&source)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.name)
        .collect();
    assert!(!names.contains(&"Cartoon, Jéja".to_string()), "{names:?}");
    assert!(
        names.contains(&"Various Artists".to_string()),
        "no shared credit keeps the byline"
    );
    let duo = db.album(&source, "duo").await.unwrap().unwrap();
    assert_eq!(
        duo.artist, "Cartoon, Jéja",
        "the byline still reads as tagged"
    );
    let billed = db
        .artist(&source, duo.artist_key.as_deref().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(billed.name, "Cartoon");

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

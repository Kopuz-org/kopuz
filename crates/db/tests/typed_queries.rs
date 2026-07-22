//! Smoke tests for the typed narrow queries that replaced `tracks_all`
//! (runtime-built SQL — not covered by the sqlx offline macro check).

use std::path::PathBuf;

use db::{Page, Source, TrackFilter, TrackSort};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Executor};

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
    let dir = std::env::temp_dir().join(format!("kopuz-tq-{pid}-{nanos}-{seq}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

async fn seed(db_path: &std::path::Path) {
    let mut conn = SqliteConnectOptions::new()
        .filename(db_path)
        .connect()
        .await
        .unwrap();
    let mut batch = String::new();
    // Two local albums (rock inserted before jazz so jazz is "newer"), one
    // server track, listen counts keyed by uid (path / "ytmusic:id").
    for (i, (key, album_id, title, artist, album, disc, track)) in [
        (
            "/music/rock/a1.flac",
            "al-rock",
            "Anthem",
            "Axel",
            "Rock One",
            1,
            2,
        ),
        (
            "/music/rock/a2.flac",
            "al-rock",
            "Ballad",
            "Axel",
            "Rock One",
            1,
            1,
        ),
        (
            "/music/jazz/b_1.flac",
            "al-jazz",
            "Cool",
            "Bea",
            "Jazz One",
            1,
            1,
        ),
        (
            "/music/jazz/b_2.flac",
            "al-jazz",
            "Drift",
            "Bea",
            "Jazz One",
            1,
            2,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        batch.push_str(&format!(
            "INSERT INTO tracks (rowid_pk, source, track_key, source_album_id, title, artist, album, \
             disc_number, track_number, artists_json) VALUES \
             ({}, 'local', '{key}', '{album_id}', '{title}', '{artist}', '{album}', {disc}, {track}, '[]');\n",
            i + 1
        ));
    }
    batch.push_str(
        "INSERT INTO tracks (rowid_pk, source, track_key, service, source_album_id, title, artist, album, artists_json) \
         VALUES (100, 'srv-1', 'vid1', 'YtMusic', 'al-yt', 'Server Song', 'Cyn', 'Yt Album', '[]');\n\
         INSERT INTO tracks (rowid_pk, source, track_key, source_album_id, title, artist, album, artists_json) \
         VALUES (101, 'local:test', '/music/jazz/b_1.flac', 'al-separate', 'Separate Song', 'Dee', 'Separate Album', '[]');\n\
         INSERT INTO albums (source, source_album_id, title, artist, genre) VALUES \
           ('local', 'al-rock', 'Rock One', 'Axel', 'Rock'), \
           ('local', 'al-jazz', 'Jazz One', 'Bea', 'Jazz'), \
           ('local:test', 'al-separate', 'Separate Album', 'Dee', 'Other'), \
           ('srv-1', 'al-yt', 'Yt Album', 'Cyn', 'Pop');\n\
         INSERT INTO listen_counts (track_key, count) VALUES \
           ('/music/rock/a1.flac', 3), ('/music/jazz/b_1.flac', 10), ('ytmusic:vid1', 7);\n",
    );
    conn.execute(batch.as_str()).await.unwrap();
}

#[tokio::test]
async fn typed_queries_smoke() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    seed(&db_path).await;
    let local = Source::Local;
    let srv = Source::Server("srv-1".into());

    let rock = db.album_tracks(&local, "al-rock").await.unwrap();
    assert_eq!(
        rock.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
        ["Ballad", "Anthem"],
        "album_tracks orders by disc/track"
    );

    let bea = db.artist_tracks(&local, "Bea", None).await.unwrap();
    assert_eq!(bea.len(), 2);
    assert!(bea.iter().all(|t| t.artist == "Bea"));

    let bounded = db.artist_tracks(&local, "Bea", Some(1)).await.unwrap();
    assert_eq!(bounded.len(), 1, "limit bounds the query SQL-side");

    let jazz = db.genre_tracks(&local, "Jazz").await.unwrap();
    assert_eq!(jazz.len(), 2);
    assert!(jazz.iter().all(|t| t.album == "Jazz One"));

    // Prefix with an underscore in a filename must not act as a wildcard.
    let folder = db
        .folder_tracks(&Source::Local, "/music/jazz/")
        .await
        .unwrap();
    assert_eq!(folder.len(), 2);
    let separate = db
        .folder_tracks(&Source::LocalLibrary("local:test".into()), "/music/jazz/")
        .await
        .unwrap();
    assert_eq!(separate.len(), 1);
    assert_eq!(separate[0].title, "Separate Song");
    let none = db
        .folder_tracks(&Source::Local, "/music/ja_z/")
        .await
        .unwrap();
    assert!(none.is_empty(), "LIKE metachars are escaped");

    let samples = db.artist_sample_tracks(&local, 10).await.unwrap();
    assert_eq!(
        samples
            .iter()
            .map(|t| t.artist.as_str())
            .collect::<Vec<_>>(),
        ["Axel", "Bea"],
        "one per artist, A→Z"
    );

    assert_eq!(
        db.top_genre(&local).await.unwrap().as_deref(),
        Some("Jazz"),
        "highest summed plays wins"
    );
    assert_eq!(
        db.top_genre(&srv).await.unwrap().as_deref(),
        Some("Pop"),
        "server uid join (service:id) maps listen counts"
    );

    let by_plays = db
        .tracks_page(
            &TrackFilter {
                source: local.clone(),
                sort: TrackSort::PlayCount,
                search: String::new(),
            },
            Page {
                offset: 0,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        by_plays
            .iter()
            .map(|t| t.title.as_str())
            .collect::<Vec<_>>(),
        ["Cool", "Anthem", "Ballad", "Drift"],
        "plays DESC, title tiebreak"
    );

    assert_eq!(db.search_corpus(&local).await.unwrap().len(), 4);
}

/// A track self-resolves its cover: a local row (NULL `cover_path`) gets its
/// album's cover via the read-layer JOIN; a server row keeps its own `cover_path`
/// even when its album has a different one (COALESCE must not clobber it).
#[tokio::test]
async fn track_cover_projects_from_album_for_local_keeps_own_for_server() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    let mut conn = SqliteConnectOptions::new()
        .filename(&db_path)
        .connect()
        .await
        .unwrap();
    conn.execute(
        "INSERT INTO albums (source, source_album_id, title, artist, genre, cover_path) VALUES \
           ('local', 'al-x', 'X', 'A', 'Rock', '/covers/al-x.jpg'), \
           ('srv-1', 'al-srv', 'SrvAlbum', 'B', 'Pop', '/album/should-not-win.jpg'); \
         INSERT INTO tracks (rowid_pk, source, track_key, source_album_id, title, artist, album, artists_json) \
           VALUES (1, 'local', '/music/x.flac', 'al-x', 'Song', 'A', 'X', '[]'); \
         INSERT INTO tracks (rowid_pk, source, track_key, service, source_album_id, title, artist, album, cover_path, artists_json) \
           VALUES (2, 'srv-1', 'vid9', 'YtMusic', 'al-srv', 'SrvSong', 'B', 'SrvAlbum', 'own-ref', '[]'); \
         INSERT INTO tracks (rowid_pk, source, track_key, service, source_album_id, title, artist, album, artists_json) \
           VALUES (3, 'srv-1', 'vid10', 'YtMusic', 'al-srv', 'SrvSongNoCover', 'B', 'SrvAlbum', '[]');",
    )
    .await
    .unwrap();

    let local = db.album_tracks(&Source::Local, "al-x").await.unwrap();
    assert_eq!(local.len(), 1);
    assert_eq!(
        local[0].cover.as_deref(),
        Some("/covers/al-x.jpg"),
        "local track's NULL cover_path falls back to its album's cover via JOIN"
    );

    let srv = db
        .album_tracks(&Source::Server("srv-1".into()), "al-srv")
        .await
        .unwrap();
    assert_eq!(srv.len(), 2);
    let with_cover = srv.iter().find(|t| t.title == "SrvSong").unwrap();
    assert_eq!(
        with_cover.cover.as_deref(),
        Some("own-ref"),
        "server track keeps its own cover ref; COALESCE doesn't pull the album's"
    );
    // The regression guard: a server track with NO own cover must stay NULL, NOT
    // inherit the album's `cover_path`. The album ref is service-encoded, and the
    // cover resolver would misread it as the track's own image tag (#cover). Server
    // rows fall back to the album via `album_id` at resolve time instead.
    let no_cover = srv.iter().find(|t| t.title == "SrvSongNoCover").unwrap();
    assert_eq!(
        no_cover.cover.as_deref(),
        None,
        "server track's NULL cover stays NULL — album cover is not projected onto server rows"
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

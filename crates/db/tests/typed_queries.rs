//! Smoke tests for the typed narrow queries that replaced `tracks_all`
//! (runtime-built SQL — not covered by the sqlx offline macro check).

use std::path::PathBuf;

use db::{Page, Source, TrackFilter, TrackSort};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Executor};

fn unique_db() -> PathBuf {
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
             disc_number, track_number) VALUES \
             ({}, 'local', '{key}', '{album_id}', '{title}', '{artist}', '{album}', {disc}, {track});\n",
            i + 1
        ));
    }
    batch.push_str(
        "INSERT INTO tracks (rowid_pk, source, track_key, service, source_album_id, title, artist, album) \
         VALUES (100, 'srv-1', 'vid1', 'YtMusic', 'al-yt', 'Server Song', 'Cyn', 'Yt Album');\n\
         INSERT INTO tracks (rowid_pk, source, track_key, source_album_id, title, artist, album) \
         VALUES (101, 'local:test', '/music/jazz/b_1.flac', 'al-separate', 'Separate Song', 'Dee', 'Separate Album');\n\
         INSERT INTO albums (source, source_album_id, title, artist, genre) VALUES \
           ('local', 'al-rock', 'Rock One', 'Axel', 'Rock'), \
           ('local', 'al-jazz', 'Jazz One', 'Bea', 'Jazz'), \
           ('local:test', 'al-separate', 'Separate Album', 'Dee', 'Other'), \
           ('srv-1', 'al-yt', 'Yt Album', 'Cyn', 'Pop');\n\
         INSERT INTO listen_counts (source, track_key, count) VALUES \
           ('local', '/music/rock/a1.flac', 3), ('local', '/music/jazz/b_1.flac', 10), ('srv-1', 'vid1', 7);\n\
         INSERT INTO artists (source, name, name_key, key) SELECT source, artist, lower(artist), lower(hex(randomblob(16))) FROM (SELECT DISTINCT source, artist FROM tracks);\n\
         INSERT INTO track_credits (track_pk, position, artist_pk, name) \
           SELECT t.rowid_pk, 0, a.id, t.artist FROM tracks t JOIN artists a ON a.source = t.source AND a.name = t.artist;\n",
    );
    conn.execute(batch.as_str()).await.unwrap();
}

#[tokio::test]
async fn album_and_genre_filters_share_sorting_and_counts() {
    let path = unique_db();
    let database = db::init(&path).await.unwrap();
    seed(&path).await;
    let filter = TrackFilter {
        source: Source::default(),
        album: Some("al-rock".into()),
        genre: Some("Rock".into()),
        sort: TrackSort::Title,
        ..Default::default()
    };
    let rows = database
        .tracks_page(
            &filter,
            Page {
                offset: 1,
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(database.tracks_count(&filter).await.unwrap(), 2);
    assert_eq!(rows[0].title, "Ballad");
    let mismatched = TrackFilter {
        genre: Some("Jazz".into()),
        ..filter
    };
    assert_eq!(database.tracks_count(&mismatched).await.unwrap(), 0);
    assert!(
        database
            .tracks_page(
                &mismatched,
                Page {
                    offset: 0,
                    limit: 10
                }
            )
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn typed_queries_smoke() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    seed(&db_path).await;
    let local = Source::default();
    let srv = Source::Server("srv-1".into());

    let rock = db.album_tracks(&local, "al-rock").await.unwrap();
    assert_eq!(
        rock.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(),
        ["Ballad", "Anthem"],
        "album_tracks orders by disc/track"
    );

    let bea_key = db
        .artists(&local)
        .await
        .unwrap()
        .into_iter()
        .find(|artist| artist.name == "Bea")
        .expect("Bea is listed")
        .key;
    let bea = db.artist_tracks(&local, &bea_key, None).await.unwrap();
    assert_eq!(bea.len(), 2);
    assert!(bea.iter().all(|t| t.artist == "Bea"));

    let bounded = db.artist_tracks(&local, &bea_key, Some(1)).await.unwrap();
    assert_eq!(bounded.len(), 1, "limit bounds the query SQL-side");

    let jazz = db.genre_tracks(&local, "Jazz").await.unwrap();
    assert_eq!(jazz.len(), 2);
    assert!(jazz.iter().all(|t| t.album == "Jazz One"));

    let folder = db
        .folder_tracks(&Source::default(), "/music/jazz/")
        .await
        .unwrap();
    assert_eq!(folder.len(), 2);
    let separate = db
        .folder_tracks(&Source::LocalLibrary("local:test".into()), "/music/jazz/")
        .await
        .unwrap();
    assert_eq!(separate.len(), 1);
    assert_eq!(separate[0].title, "Separate Song");
    assert_eq!(
        separate[0].id.local_path(),
        Some(std::path::Path::new("/music/jazz/b_1.flac")),
        "named local sources must reconstruct filesystem track ids",
    );
    let none = db
        .folder_tracks(&Source::default(), "/music/ja_z/")
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
                favorite: None,
                ..Default::default()
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
         INSERT INTO tracks (rowid_pk, source, track_key, source_album_id, title, artist, album) \
           VALUES (1, 'local', '/music/x.flac', 'al-x', 'Song', 'A', 'X'); \
         INSERT INTO tracks (rowid_pk, source, track_key, service, source_album_id, title, artist, album, cover_path) \
           VALUES (2, 'srv-1', 'vid9', 'YtMusic', 'al-srv', 'SrvSong', 'B', 'SrvAlbum', 'own-ref'); \
         INSERT INTO tracks (rowid_pk, source, track_key, service, source_album_id, title, artist, album) \
           VALUES (3, 'srv-1', 'vid10', 'YtMusic', 'al-srv', 'SrvSongNoCover', 'B', 'SrvAlbum');",
    )
    .await
    .unwrap();

    let local = db.album_tracks(&Source::default(), "al-x").await.unwrap();
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

    let no_cover = srv.iter().find(|t| t.title == "SrvSongNoCover").unwrap();
    assert_eq!(
        no_cover.cover.as_deref(),
        None,
        "server track's NULL cover stays NULL — album cover is not projected onto server rows"
    );

    let _ = std::fs::remove_dir_all(db_path.parent().unwrap());
}

/// The favorite filter runs in SQL so paging and totals stay correct on a
/// large library. favorites.server_id carries the same string as
/// tracks.source, and favorites.ref carries the track_key.
#[tokio::test]
async fn track_filter_selects_favorites_in_sql() {
    let path = unique_db();
    let db = db::init(&path).await.unwrap();
    seed(&path).await;
    let local = Source::default();

    db.set_favorite(local.as_str(), "/music/jazz/b_1.flac", true)
        .await
        .unwrap();
    db.set_favorite(local.as_str(), "/music/rock/a1.flac", true)
        .await
        .unwrap();

    let filter = |favorite| TrackFilter {
        source: local.clone(),
        sort: TrackSort::Title,
        search: String::new(),
        favorite,
        ..Default::default()
    };
    let page = Page {
        offset: 0,
        limit: 100,
    };

    let favorites = db.tracks_page(&filter(Some(true)), page).await.unwrap();
    let mut keys: Vec<String> = favorites
        .iter()
        .map(|track| track.id.key().to_string())
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["/music/jazz/b_1.flac", "/music/rock/a1.flac"]);

    assert_eq!(db.tracks_count(&filter(Some(true))).await.unwrap(), 2);

    let others = db.tracks_page(&filter(Some(false)), page).await.unwrap();
    assert!(
        others
            .iter()
            .all(|track| !keys.contains(&track.id.key().to_string())),
        "Some(false) is the complement, not everything"
    );

    let unfiltered = db.tracks_count(&filter(None)).await.unwrap();
    assert_eq!(
        unfiltered,
        2 + others.len() as u32,
        "the two halves partition the library"
    );
}

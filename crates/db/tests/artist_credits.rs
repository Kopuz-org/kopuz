//! A library scanned before the credit splitter existed still has to lose its
//! phantom "A$AP Rocky feat. Drake" artist, without a rescan.

use std::path::PathBuf;

use db::Source;
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
    let dir = std::env::temp_dir().join(format!("kopuz-ac-{pid}-{nanos}-{seq}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("kopuz.db")
}

/// Write rows the way the pre-split ingest did (the whole credit as one
/// artist), then clear the marker so the next open backfills them.
async fn seed_pre_split(db_path: &std::path::Path, rows: &[(&str, &str)]) {
    let mut conn = SqliteConnectOptions::new()
        .filename(db_path)
        .connect()
        .await
        .unwrap();
    conn.execute("BEGIN").await.unwrap();
    for (i, (key, artist)) in rows.iter().enumerate() {
        let artists_json = serde_json::to_string(&[artist]).unwrap();
        sqlx::query(
            "INSERT INTO tracks (source, track_key, title, artist, album, artists_json) \
             VALUES ('local', ?1, ?2, ?3, 'Album', ?4)",
        )
        .bind(key)
        .bind(format!("Track {i}"))
        .bind(artist)
        .bind(&artists_json)
        .execute(&mut conn)
        .await
        .unwrap();
    }
    conn.execute("DELETE FROM metadata_cache WHERE cache_key = 'artist_credits'")
        .await
        .unwrap();
    conn.execute("COMMIT").await.unwrap();
}

async fn stored_credits(db_path: &std::path::Path, track_key: &str) -> Vec<String> {
    let mut conn = SqliteConnectOptions::new()
        .filename(db_path)
        .connect()
        .await
        .unwrap();
    let json: String = sqlx::query_scalar("SELECT artists_json FROM tracks WHERE track_key = ?1")
        .bind(track_key)
        .fetch_one(&mut conn)
        .await
        .unwrap();
    serde_json::from_str(&json).unwrap()
}

#[tokio::test]
async fn backfill_splits_joined_credits_without_a_rescan() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();
    drop(db);

    seed_pre_split(
        &db_path,
        &[
            ("/music/1.flac", "A$AP Rocky"),
            ("/music/2.flac", "A$AP Rocky feat. Drake"),
            ("/music/3.flac", "A$AP Rocky ft. Tyler, The Creator"),
            ("/music/4.flac", "Earth, Wind & Fire"),
        ],
    )
    .await;

    let db = db::init(&db_path).await.unwrap();

    assert_eq!(
        stored_credits(&db_path, "/music/1.flac").await,
        ["A$AP Rocky"]
    );
    assert_eq!(
        stored_credits(&db_path, "/music/2.flac").await,
        ["A$AP Rocky", "Drake"]
    );
    assert_eq!(
        stored_credits(&db_path, "/music/3.flac").await,
        ["A$AP Rocky", "Tyler, The Creator"]
    );
    // A real name that only looks like a join is left exactly as it was.
    assert_eq!(
        stored_credits(&db_path, "/music/4.flac").await,
        ["Earth, Wind & Fire"]
    );

    let artists = db.artists(&Source::Local).await.unwrap();
    let names: Vec<&str> = artists.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "A$AP Rocky",
            "Drake",
            "Earth, Wind & Fire",
            "Tyler, The Creator"
        ]
    );

    // The primary is counted on every track that credits them, not only the
    // ones where the joined string happened to match exactly.
    let count = |name: &str| {
        artists
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, c)| *c)
            .unwrap_or(0)
    };
    assert_eq!(count("A$AP Rocky"), 3);
    assert_eq!(count("Drake"), 1);
    assert_eq!(count("Tyler, The Creator"), 1);
    assert_eq!(count("Earth, Wind & Fire"), 1);
}

#[tokio::test]
async fn backfill_runs_once_per_database() {
    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());

    seed_pre_split(&db_path, &[("/music/1.flac", "A feat. B")]).await;
    drop(db::init(&db_path).await.unwrap());
    assert_eq!(stored_credits(&db_path, "/music/1.flac").await, ["A", "B"]);

    // A later hand edit is not undone by a second open.
    let mut conn = SqliteConnectOptions::new()
        .filename(&db_path)
        .connect()
        .await
        .unwrap();
    conn.execute("UPDATE tracks SET artists_json = '[\"Kept\"]'")
        .await
        .unwrap();
    drop(conn);

    drop(db::init(&db_path).await.unwrap());
    assert_eq!(stored_credits(&db_path, "/music/1.flac").await, ["Kept"]);
}

#[tokio::test]
async fn artists_falls_back_to_the_joined_column_when_no_credits_are_stored() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    let mut conn = SqliteConnectOptions::new()
        .filename(&db_path)
        .connect()
        .await
        .unwrap();
    conn.execute(
        "INSERT INTO tracks (source, track_key, title, artist, album, artists_json) \
         VALUES ('local', '/music/1.flac', 'T', 'Solo Artist', 'Album', '[]')",
    )
    .await
    .unwrap();
    drop(conn);

    let artists = db.artists(&Source::Local).await.unwrap();
    assert_eq!(artists, [("Solo Artist".to_string(), 1)]);
}

/// The case a user on the previous build is actually in: the first pass already
/// ran, stored its partial split, and burned the marker. The new rules have to
/// reach them anyway, and have to work from the untouched `artist` column,
/// because the first pass flattened the head/tail shape the contributor-list
/// rule reads.
#[tokio::test]
async fn a_library_backfilled_by_the_previous_revision_is_re_split() {
    const CREDIT: &str = "A$AP Rocky feat. Joe Fox x Future x M.I.A. \u{2022} A$AP Rocky \u{2022} \
                          Joe Fox \u{2022} Future \u{2022} M.I.A. \u{2022} Rakim Mayers \u{2022} \
                          Rameses Magnus-George \u{2022} Axel Morgan \u{2022} Ricci Rierra \u{2022} \
                          Nayvadius Wilburn";

    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());

    let mut conn = SqliteConnectOptions::new()
        .filename(&db_path)
        .connect()
        .await
        .unwrap();
    // Exactly what the previous revision left behind: feat/x resolved, the
    // bullet tail still welded into one entry.
    let previous = serde_json::to_string(&[
        "A$AP Rocky",
        "Joe Fox",
        "Future",
        "M.I.A. \u{2022} A$AP Rocky \u{2022} Joe Fox \u{2022} Future \u{2022} M.I.A. \u{2022} \
         Rakim Mayers \u{2022} Rameses Magnus-George \u{2022} Axel Morgan \u{2022} Ricci Rierra \
         \u{2022} Nayvadius Wilburn",
    ])
    .unwrap();
    sqlx::query(
        "INSERT INTO tracks (source, track_key, title, artist, album, artists_json) \
         VALUES ('local', '/music/1.flac', 'T', ?1, 'Album', ?2)",
    )
    .bind(CREDIT)
    .bind(&previous)
    .execute(&mut conn)
    .await
    .unwrap();
    // Leave exactly the marker the previous revision wrote, which must not
    // block the new one.
    sqlx::query("DELETE FROM metadata_cache WHERE cache_key = 'artist_credits'")
        .execute(&mut conn)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO metadata_cache (cache_key, kind, payload) \
         VALUES ('artist_credits', 'split', '1')",
    )
    .execute(&mut conn)
    .await
    .unwrap();
    drop(conn);

    drop(db::init(&db_path).await.unwrap());

    assert_eq!(
        stored_credits(&db_path, "/music/1.flac").await,
        ["A$AP Rocky", "Joe Fox", "Future", "M.I.A."]
    );

    // One marker row, at the new revision: the superseded one is cleared out.
    let mut conn = SqliteConnectOptions::new()
        .filename(&db_path)
        .connect()
        .await
        .unwrap();
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM metadata_cache WHERE cache_key = 'artist_credits'")
            .fetch_all(&mut conn)
            .await
            .unwrap();
    assert_eq!(kinds, ["v5-joins-by-evidence"]);
}

/// A per-artist list richer than the credit string (Jellyfin's `Artists` array
/// against a joined display name) is not thrown away by re-deriving.
#[tokio::test]
async fn a_source_supplied_list_survives_the_backfill() {
    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());

    let mut conn = SqliteConnectOptions::new()
        .filename(&db_path)
        .connect()
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO tracks (source, track_key, title, artist, album, artists_json) \
         VALUES ('local', '/music/1.flac', 'T', 'Gorillaz', 'Album', ?1)",
    )
    .bind(serde_json::to_string(&["Gorillaz", "Del The Funky Homosapien"]).unwrap())
    .execute(&mut conn)
    .await
    .unwrap();
    sqlx::query("DELETE FROM metadata_cache WHERE cache_key = 'artist_credits'")
        .execute(&mut conn)
        .await
        .unwrap();
    drop(conn);

    drop(db::init(&db_path).await.unwrap());

    assert_eq!(
        stored_credits(&db_path, "/music/1.flac").await,
        ["Gorillaz", "Del The Funky Homosapien"]
    );
}

/// Seed a library from bare credit strings, leaving no marker behind so the
/// next open backfills it.
async fn seed_credits(db_path: &std::path::Path, credits: &[&str]) {
    let mut conn = SqliteConnectOptions::new()
        .filename(db_path)
        .connect()
        .await
        .unwrap();
    conn.execute("BEGIN").await.unwrap();
    for (i, credit) in credits.iter().enumerate() {
        sqlx::query(
            "INSERT INTO tracks (source, track_key, title, artist, album, artists_json) \
             VALUES ('local', ?1, ?2, ?3, 'Album', '[]')",
        )
        .bind(format!("/music/{i}.flac"))
        .bind(format!("Track {i}"))
        .bind(credit)
        .execute(&mut conn)
        .await
        .unwrap();
    }
    conn.execute("DELETE FROM metadata_cache WHERE cache_key = 'artist_credits'")
        .await
        .unwrap();
    conn.execute("COMMIT").await.unwrap();
}

async fn artist_names(db: &db::Db) -> Vec<String> {
    db.artists(&Source::Local)
        .await
        .unwrap()
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

/// Every piece attested on its own is proof enough, so all of them are kept.
#[tokio::test]
async fn a_fully_attested_comma_credit_splits_into_every_piece() {
    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());
    seed_credits(
        &db_path,
        &[
            "12th Planet, Kill The Noise, Skrillex",
            "12th Planet",
            "Kill The Noise",
            "Skrillex",
        ],
    )
    .await;
    let db = db::init(&db_path).await.unwrap();

    assert_eq!(
        artist_names(&db).await,
        ["12th Planet", "Kill The Noise", "Skrillex"]
    );
}

/// The names the evidence rule has to leave alone. Neither "The Creator" nor
/// "Wind" nor "Stills & Nash" is an artist anywhere in the library, so nothing
/// licenses a split.
#[tokio::test]
async fn a_comma_inside_a_real_name_is_left_whole() {
    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());
    seed_credits(
        &db_path,
        &[
            "Tyler, The Creator",
            "Earth, Wind & Fire",
            "Crosby, Stills & Nash",
            "Emerson, Lake & Palmer",
        ],
    )
    .await;
    let db = db::init(&db_path).await.unwrap();

    assert_eq!(
        artist_names(&db).await,
        [
            "Crosby, Stills & Nash",
            "Earth, Wind & Fire",
            "Emerson, Lake & Palmer",
            "Tyler, The Creator"
        ]
    );
}

/// End to end over the shapes the user's library actually contains.
#[tokio::test]
async fn the_reported_library_shapes_resolve_to_one_tile_each() {
    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());
    seed_credits(
        &db_path,
        &[
            "A$AP Rocky",
            "A$AP Rocky • Rakim Mayers",
            "A$AP Rocky • Bones • Frans Mernick • Hector Delgado • Rakim Mayers",
            "A$AP Rocky/ James Fauntleroy/ James Fauntleroy",
            "A$AP Rocky/ Joe Fox",
            "A$AP Rocky feat. ScHoolboy Q",
        ],
    )
    .await;
    let db = db::init(&db_path).await.unwrap();

    // No "Rakim Mayers", "Hector Delgado" or "Bones": the tail of a bullet
    // list. No "James Fauntleroy" or "Joe Fox" either, because neither carries
    // a track on its own anywhere here, so the slash credits hand back only
    // the piece the library attests. "ScHoolboy Q" stays: a featured artist is
    // named by the string itself and needs no corroboration.
    assert_eq!(artist_names(&db).await, ["A$AP Rocky", "ScHoolboy Q"]);

    let counts = db.artists(&Source::Local).await.unwrap();
    let rocky = counts.iter().find(|(n, _)| n == "A$AP Rocky").unwrap().1;
    assert_eq!(rocky, 6, "every credit files under A$AP Rocky");
}

/// The regression that made this revision necessary. "Tyler, The Creator" is
/// the whole artist field on many tracks, and "Tyler" also turns up as a
/// fragment inside several other joined credits. The fragments count for
/// nothing; the credit carrying whole tracks on its own is what decides.
#[tokio::test]
async fn a_comma_name_on_many_tracks_survives_its_fragments_recurring() {
    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());
    seed_credits(
        &db_path,
        &[
            "Tyler, The Creator",
            "Tyler, The Creator",
            "Tyler, The Creator",
            "Tyler, The Creator",
            "Tyler, The Creator",
            // "Tyler" now recurs beside three different partners, which the
            // previous revision read as proof it was a standalone artist.
            "Tyler, The Creator & Frank Ocean",
            "Tyler, The Creator & Kali Uchis",
            "Tyler, The Creator & Lil Wayne",
        ],
    )
    .await;
    let db = db::init(&db_path).await.unwrap();

    let names = artist_names(&db).await;
    assert!(
        !names.iter().any(|n| n == "Tyler"),
        "no bare Tyler tile, got {names:?}"
    );
    assert!(
        !names.iter().any(|n| n == "The Creator"),
        "no bare The Creator tile, got {names:?}"
    );
    assert!(names.iter().any(|n| n == "Tyler, The Creator"));

    let counts = db.artists(&Source::Local).await.unwrap();
    let whole = counts
        .iter()
        .find(|(n, _)| n == "Tyler, The Creator")
        .unwrap()
        .1;
    assert_eq!(whole, 5, "the plain credit keeps all of its own tracks");

    // The "& ..." variants are their own credits and stay whole: an ampersand
    // is never a separator, and no piece of them stands alone to license one.
    assert!(
        names
            .iter()
            .any(|n| n == "Tyler, The Creator & Frank Ocean")
    );
}

/// A piece that carries whole tracks on its own is a real artist, so the
/// one-off collaborations it appears in collapse onto it.
#[tokio::test]
async fn a_credit_that_stands_alone_elsewhere_is_split_out() {
    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());
    seed_credits(
        &db_path,
        &[
            "49th & Main",
            "49th & Main, A Little Sound",
            "49th & Main, Brandon Nembhard",
            "49th & Main, SHEE",
        ],
    )
    .await;
    let db = db::init(&db_path).await.unwrap();

    assert_eq!(artist_names(&db).await, ["49th & Main"]);
    let counts = db.artists(&Source::Local).await.unwrap();
    assert_eq!(counts[0].1, 4);
}

/// Without a standalone track anywhere, nothing licenses the split and the
/// credit is left alone. Under-splitting is the cheap direction.
#[tokio::test]
async fn a_collaboration_whose_pieces_never_stand_alone_is_left_whole() {
    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());
    seed_credits(
        &db_path,
        &[
            "49th & Main, A Little Sound",
            "49th & Main, Brandon Nembhard",
            "49th & Main, SHEE",
        ],
    )
    .await;
    let db = db::init(&db_path).await.unwrap();

    assert_eq!(
        artist_names(&db).await,
        [
            "49th & Main, A Little Sound",
            "49th & Main, Brandon Nembhard",
            "49th & Main, SHEE"
        ]
    );
}

/// Head-only has to reach a row whose stored list is an older pass's full
/// split of the same bullet string. Deferring to the longer list is what kept
/// "Rakim Mayers" and "Hector Delgado" alive.
#[tokio::test]
async fn a_stored_personnel_list_does_not_outrank_the_credit_string() {
    const CREDIT: &str = "A$AP Rocky \u{2022} Bones \u{2022} Frans Mernick \u{2022} Hector Delgado \u{2022} \
         Rakim Mayers \u{2022} Elmo O'Connor";

    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());

    let mut conn = SqliteConnectOptions::new()
        .filename(&db_path)
        .connect()
        .await
        .unwrap();
    let previous = serde_json::to_string(&[
        "A$AP Rocky",
        "Bones",
        "Frans Mernick",
        "Hector Delgado",
        "Rakim Mayers",
        "Elmo O'Connor",
    ])
    .unwrap();
    sqlx::query(
        "INSERT INTO tracks (source, track_key, title, artist, album, artists_json) \
         VALUES ('local', '/music/1.flac', 'T', ?1, 'Album', ?2)",
    )
    .bind(CREDIT)
    .bind(&previous)
    .execute(&mut conn)
    .await
    .unwrap();
    sqlx::query("DELETE FROM metadata_cache WHERE cache_key = 'artist_credits'")
        .execute(&mut conn)
        .await
        .unwrap();
    drop(conn);

    let db = db::init(&db_path).await.unwrap();
    assert_eq!(artist_names(&db).await, ["A$AP Rocky"]);
}

/// The names that must come through every rule untouched.
#[tokio::test]
async fn real_names_survive_the_whole_pipeline() {
    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());
    seed_credits(
        &db_path,
        &[
            "Tyler, The Creator",
            "Earth, Wind & Fire",
            "Crosby, Stills & Nash",
            "Emerson, Lake & Palmer",
            "AC/DC",
            "&ME",
            "Simon & Garfunkel",
            "Florence + the Machine",
        ],
    )
    .await;
    let db = db::init(&db_path).await.unwrap();

    assert_eq!(
        artist_names(&db).await,
        [
            "&ME",
            "AC/DC",
            "Crosby, Stills & Nash",
            "Earth, Wind & Fire",
            "Emerson, Lake & Palmer",
            "Florence + the Machine",
            "Simon & Garfunkel",
            "Tyler, The Creator"
        ]
    );
}

/// The five names Jellyfin protects with a hardcoded table. None of them is
/// listed here: each survives because it is the whole artist field on its own
/// tracks and no piece of it stands alone anywhere, which is the same reason
/// "Tyler, The Creator" survives.
#[tokio::test]
async fn real_names_holding_a_separator_survive_without_a_name_table() {
    const NAMES: [&str; 5] = [
        "We;Na",
        "Kairon; IRSE!",
        "R!N / Gemie",
        "LOONA / yyxy",
        "LOONA / ODD EYE CIRCLE",
    ];

    // Three tracks each, so every one clears the whole-credit threshold the
    // way a real artist's catalogue does.
    let credits: Vec<&str> = NAMES.iter().flat_map(|n| [*n, *n, *n]).collect();

    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());
    seed_credits(&db_path, &credits).await;
    let db = db::init(&db_path).await.unwrap();

    let names = artist_names(&db).await;
    for name in NAMES {
        assert!(
            names.iter().any(|n| n == name),
            "{name:?} lost, got {names:?}"
        );
    }
    // No fragment became a tile of its own.
    for fragment in [
        "We",
        "Na",
        "Kairon",
        "IRSE!",
        "R!N",
        "Gemie",
        "LOONA",
        "yyxy",
        "ODD EYE CIRCLE",
    ] {
        assert!(
            !names.iter().any(|n| n == fragment),
            "{fragment:?} should not be a tile, got {names:?}"
        );
    }
}

/// The opposite direction, and the reason a name table is not enough: the same
/// shapes DO split when the library shows the pieces standing on their own.
#[tokio::test]
async fn a_joined_credit_splits_when_its_pieces_stand_alone() {
    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());
    seed_credits(
        &db_path,
        &[
            "A$AP Rocky",
            "Joe Fox",
            "Daft Punk",
            "Pharrell Williams",
            "A$AP Rocky/ Joe Fox",
            "Daft Punk;Pharrell Williams",
        ],
    )
    .await;
    let db = db::init(&db_path).await.unwrap();

    assert_eq!(
        artist_names(&db).await,
        ["A$AP Rocky", "Daft Punk", "Joe Fox", "Pharrell Williams"]
    );

    let counts = db.artists(&Source::Local).await.unwrap();
    let of = |name: &str| counts.iter().find(|(n, _)| n == name).unwrap().1;
    assert_eq!(of("A$AP Rocky"), 2);
    assert_eq!(of("Joe Fox"), 2);
    assert_eq!(of("Daft Punk"), 2);
    assert_eq!(of("Pharrell Williams"), 2);
}

/// A padded slash on few tracks whose pieces never stand alone stays whole:
/// with no evidence either way, the join is not assumed.
#[tokio::test]
async fn a_rare_slash_credit_with_no_evidence_is_left_whole() {
    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());
    seed_credits(&db_path, &["R!N / Gemie"]).await;
    let db = db::init(&db_path).await.unwrap();

    assert_eq!(artist_names(&db).await, ["R!N / Gemie"]);
}

/// "AC/DC" never even reaches the evidence test, so a library that happens to
/// hold a solo "AC" and a solo "DC" still cannot break it.
#[tokio::test]
async fn an_unpadded_slash_is_never_a_candidate() {
    let db_path = unique_db();
    drop(db::init(&db_path).await.unwrap());
    seed_credits(&db_path, &["AC/DC", "AC", "DC"]).await;
    let db = db::init(&db_path).await.unwrap();

    let names = artist_names(&db).await;
    assert!(names.iter().any(|n| n == "AC/DC"), "got {names:?}");
    let counts = db.artists(&Source::Local).await.unwrap();
    assert_eq!(counts.iter().find(|(n, _)| n == "AC/DC").unwrap().1, 1);
}

/// The fallback branch reads `artist` straight from a legacy row, so it has to
/// trim it the way the credit list is trimmed. Otherwise a padded value becomes
/// an artist of its own next to the clean spelling, and a whitespace-only value
/// becomes a blank tile.
#[tokio::test]
async fn the_fallback_branch_trims_the_joined_column() {
    let db_path = unique_db();
    let db = db::init(&db_path).await.unwrap();

    let mut conn = SqliteConnectOptions::new()
        .filename(&db_path)
        .connect()
        .await
        .unwrap();
    for (key, artist) in [
        ("/music/1.flac", "Drake"),
        ("/music/2.flac", "  Drake  "),
        ("/music/3.flac", "\t\n "),
        ("/music/4.flac", ""),
    ] {
        sqlx::query(
            "INSERT INTO tracks (source, track_key, title, artist, album, artists_json) \
             VALUES ('local', ?1, 'T', ?2, 'Album', '[]')",
        )
        .bind(key)
        .bind(artist)
        .execute(&mut conn)
        .await
        .unwrap();
    }
    drop(conn);

    // One Drake carrying both rows, and no blank tile from the whitespace-only
    // value that "artist != ''" would have let through.
    assert_eq!(
        db.artists(&Source::Local).await.unwrap(),
        [("Drake".to_string(), 2)]
    );
}

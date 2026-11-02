use super::*;
use crate::backend::migrations::run_migrations;

#[test]
fn original_history_checksums_are_unchanged() {
    let expected = [
        (
            20260922000000,
            "b161c861fac58dd7f47ded06d481ff90f6047bd2af50d8c954e785be58dcc01367135cc31601b1d95453be9aa73b2335",
        ),
        (
            20260922000001,
            "26a320af17aac9cc6943fe1bcaef81e2223887f816cc2b3b9aa21543a19af833ab8c8ed61eb41321c089bbc3c678f2f2",
        ),
        (
            20260923000000,
            "02f0b9798c7b4a63a885805a4f88a7cea5e19ebb4bb2250ade940cdd44e43927c7093574d1158ec011f82e3d4459f80f",
        ),
        (
            20260924000000,
            "44836f13a798f7dd4c5e4c6d9c13bc68d8cd8fde1f74c33c3bb2efb51c47cccf5bfe7d7893721af0d08dbb68e0e092ea",
        ),
        (
            20260924000003,
            "5e6a9964238f3c74c8058a2eb5ea3fb83a02d50ff8a8181adf47c98e97622f9476b7cf2c25ee9c8da64decec37206036",
        ),
    ];
    for (version, checksum) in expected {
        let migration = LEGACY.iter().find(|m| m.version == version).unwrap();
        let actual: String = migration
            .checksum
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            actual, checksum,
            "historical migration {version} was edited"
        );
    }
}

async fn historical_pool() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    through(&pool, 20260924000003).await;
    sqlx::raw_sql(
        r#"
        INSERT INTO servers (id, name, url, service) VALUES
            ('srv', 'Server', 'https://example.test', 'Jellyfin'),
            ('other', 'Other', 'https://other.test', 'Jellyfin');
        INSERT INTO server_credentials (server_id, access_token, user_id)
            VALUES ('srv', 'test-token', 'test-user');
        INSERT INTO tracks (rowid_pk, source, track_key, title, artist) VALUES
            (1, 'srv', 'one', 'First', 'Ada'),
            (2, 'srv', 'two', 'Second', 'Ada'),
            (3, 'local', '/three', 'Third', 'Émilie'),
            (4, 'local', '/four', 'Fourth', 'ÉMILIE'),
            (5, 'other', 'one', 'Fifth', 'Ada');
        INSERT INTO track_credits (track_pk, position, name, artist_id) VALUES
            (1, 0, 'Ada', 'artist-a'), (1, 1, 'Guest', NULL),
            (2, 0, 'Ada', 'artist-b'), (3, 0, 'Émilie', NULL),
            (4, 0, 'ÉMILIE', '  '), (5, 0, 'Ada', 'artist-a');
        INSERT INTO albums (source, source_album_id, title, artist, artist_id, manual_cover, cover_path)
            VALUES ('srv', 'album', 'Album', 'Ensemble', 'ensemble', 1, '/custom-cover.png');
        INSERT INTO track_musicbrainz (track_pk, recording_id) VALUES (1, 'recording');
        INSERT INTO artist_images (artist_norm, kind, image_ref) VALUES
            ('id:srv:artist-a', 'custom', '/ada.png'), ('émilie', 'custom', '/emilie.png');
        INSERT INTO playlists (rowid_pk, source, source_pl_id, name)
            VALUES (1, 'srv', 'playlist', 'Saved');
        INSERT INTO playlist_tracks (playlist_pk, position, track_ref, item_id)
            VALUES (1, 0, 'one', 'entry-1'), (1, 1, 'two', 'entry-2');
        INSERT INTO favorites (server_id, ref, dirty, rank, epoch) VALUES ('srv', 'one', 1, 7, 2);
        INSERT INTO listen_counts (source, track_key, count) VALUES ('srv', 'one', 42);
        INSERT INTO app_config (id, json) VALUES
            (1, '{"theme":"dark","volume":0.3,"active_source":{"Server":"srv"}}');
        INSERT INTO queue_state (id, queue_json, progress_secs, shuffle_order_json, shuffle_enabled)
            VALUES (1, '[{"id":{"Server":{"service":"Jellyfin","item_id":"one"}},"album_id":"album","title":"First","artist":"Ada","album":"Album","duration":120,"khz":44,"credits":[{"name":"Ada","id":"artist-a"}]}]', 12, '[0]', 1);
        "#,
    )
    .execute(&pool)
    .await
    .unwrap();
    pool
}

async fn through(pool: &SqlitePool, version: i64) {
    let mut history = migrator();
    history.migrations.to_mut().retain(|m| m.version <= version);
    history.run(pool).await.unwrap();
}

async fn ledger(pool: &SqlitePool) -> Vec<(i64, Vec<u8>)> {
    sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn upgrades_before_credits_were_normalized() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    through(&pool, 20260922000000).await;
    sqlx::raw_sql(
        r#"INSERT INTO tracks (source, track_key, title, artist, credits_json)
           VALUES ('local', '/one', 'First', 'Ada', '[{"name":"Ada","id":"artist-a"}]');"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    run_migrations(&pool, None).await.unwrap();
    let credit: (String, String) = sqlx::query_as(
        "SELECT c.name, a.source_artist_id FROM track_credits c JOIN artists a ON a.id = c.artist_pk",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(credit, ("Ada".into(), "artist-a".into()));
}

#[tokio::test]
async fn historical_and_fresh_databases_have_the_same_columns() {
    let historical = historical_pool().await;
    let fresh = SqlitePool::connect("sqlite::memory:").await.unwrap();
    run_migrations(&historical, None).await.unwrap();
    run_migrations(&fresh, None).await.unwrap();
    let columns = "SELECT m.name, p.name, p.type, p.\"notnull\", p.dflt_value, p.pk \
                     FROM sqlite_master m, pragma_table_info(m.name) p \
                    WHERE m.type IN ('table', 'view') ORDER BY m.name, p.name";
    type Column = (String, String, String, bool, Option<String>, i64);
    let old: Vec<Column> = sqlx::query_as(columns)
        .fetch_all(&historical)
        .await
        .unwrap();
    let new: Vec<Column> = sqlx::query_as(columns).fetch_all(&fresh).await.unwrap();
    assert_eq!(old, new);
}

#[tokio::test]
async fn upgrades_historical_library_without_rewriting_its_ledger() {
    let pool = historical_pool().await;
    let before = ledger(&pool).await;
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("settings.toml");
    run_migrations(&pool, Some(&settings)).await.unwrap();
    let after = ledger(&pool).await;
    assert!(before.iter().all(|row| after.contains(row)));

    let credits: Vec<(i64, i64, String, String, Option<String>)> = sqlx::query_as(
        "SELECT c.track_pk, c.position, c.name, a.source, a.source_artist_id \
           FROM track_credits c JOIN artists a ON a.id = c.artist_pk \
          ORDER BY c.track_pk, c.position",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        credits,
        vec![
            (1, 0, "Ada".into(), "srv".into(), Some("artist-a".into())),
            (1, 1, "Guest".into(), "srv".into(), None),
            (2, 0, "Ada".into(), "srv".into(), Some("artist-b".into())),
            (3, 0, "Émilie".into(), "local".into(), None),
            (4, 0, "ÉMILIE".into(), "local".into(), None),
            (5, 0, "Ada".into(), "other".into(), Some("artist-a".into())),
        ]
    );
    let unlinked: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artists WHERE source = 'local'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(unlinked, 1);
    let album: (String, String, bool, String) = sqlx::query_as(
        "SELECT al.title, a.key, al.manual_cover, al.cover_path FROM albums al \
           JOIN artists a ON a.id = al.artist_pk",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        album,
        (
            "Album".into(),
            "ensemble".into(),
            true,
            "/custom-cover.png".into()
        )
    );
    let images: Vec<(String, String)> = sqlx::query_as(
        "SELECT a.source, i.image_ref FROM artist_images i \
           JOIN artists a ON a.source = i.source AND a.key = i.artist_key ORDER BY a.source",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        images,
        [
            ("local".into(), "/emilie.png".into()),
            ("srv".into(), "/ada.png".into())
        ]
    );
    let entries: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT position, track_ref, item_id FROM playlist_tracks ORDER BY position",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        entries,
        [
            (0, "one".into(), "entry-1".into()),
            (1, "two".into(), "entry-2".into())
        ]
    );
    let favorite: (String, i64, i64, i64) =
        sqlx::query_as("SELECT ref, dirty, rank, epoch FROM favorites")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(favorite, ("one".into(), 1, 7, 2));
    let credentials: (String, String) =
        sqlx::query_as("SELECT access_token, user_id FROM server_credentials")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(credentials, ("test-token".into(), "test-user".into()));
    let plays: i64 = sqlx::query_scalar("SELECT count FROM listen_counts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(plays, 42);
    let recording: String = sqlx::query_scalar("SELECT recording_id FROM track_musicbrainz")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(recording, "recording");
    let queue: (String, String, i64) = sqlx::query_as(
        "SELECT t.source, t.track_key, s.progress_secs FROM queue_tracks t JOIN queue_state s USING (source)",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(queue, ("srv".into(), "one".into(), 12));
    let credit: (String, String) =
        sqlx::query_as("SELECT name, source_artist_id FROM queue_credits")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(credit, ("Ada".into(), "artist-a".into()));
    let volume: f64 = sqlx::query_scalar("SELECT volume FROM app_state")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!((volume - 0.3).abs() < 1e-6);
    let saved: toml::Value = std::fs::read_to_string(&settings).unwrap().parse().unwrap();
    assert_eq!(saved["theme"].as_str(), Some("dark"));
    let violations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pragma_foreign_key_check")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(violations, 0);

    let keys: Vec<String> = sqlx::query_scalar("SELECT key FROM artists ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    run_migrations(&pool, Some(&settings)).await.unwrap();
    assert_eq!(ledger(&pool).await, after);
    let reopened: Vec<String> = sqlx::query_scalar("SELECT key FROM artists ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(reopened, keys);
}

#[tokio::test]
async fn failed_fill_rolls_back_and_retries_from_the_old_credits() {
    let pool = historical_pool().await;
    through(&pool, 20260924000004).await;
    sqlx::raw_sql(
        "CREATE TRIGGER fail_credit BEFORE INSERT ON track_credits WHEN NEW.track_pk = 2 \
         BEGIN SELECT RAISE(ABORT, 'interrupted fill'); END;",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(run_migrations(&pool, None).await.is_err());
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM legacy_track_credits), \
                (SELECT COUNT(*) FROM track_credits), (SELECT COUNT(*) FROM artists)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts, (6, 0, 0));
    assert!(!applied(&pool, 20260924000005).await.unwrap());
    sqlx::query("DROP TRIGGER fail_credit")
        .execute(&pool)
        .await
        .unwrap();
    run_migrations(&pool, None).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM track_credits")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 6);
}

#[tokio::test]
async fn unknown_checksum_is_rejected_before_the_bridge_changes_schema() {
    let pool = historical_pool().await;
    sqlx::query(
        "UPDATE _sqlx_migrations SET checksum = zeroblob(48) WHERE version = 20260924000003",
    )
    .execute(&pool)
    .await
    .unwrap();
    let before = ledger(&pool).await;
    let error = run_migrations(&pool, None).await.unwrap_err();
    assert!(error.to_string().contains("20260924000003"));
    assert_eq!(ledger(&pool).await, before);
    let artists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE name = 'artists'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(artists, 0);
}

#[tokio::test]
async fn historical_crlf_checksums_reconcile_to_the_historical_sql() {
    use sha2::{Digest, Sha384};

    let pool = historical_pool().await;
    for m in migrator().iter().filter(|m| m.version <= 20260924000003) {
        let crlf = m.sql.replace("\r\n", "\n").replace('\n', "\r\n");
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ?1 WHERE version = ?2")
            .bind(Sha384::digest(crlf.as_bytes()).to_vec())
            .bind(m.version)
            .execute(&pool)
            .await
            .unwrap();
    }
    run_migrations(&pool, None).await.unwrap();
    let expected: Vec<_> = migrator()
        .iter()
        .map(|m| (m.version, m.checksum.to_vec()))
        .collect();
    assert_eq!(ledger(&pool).await, expected);
}

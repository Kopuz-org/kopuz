//! The per-listen log, and the recap read over a window of it.

use config::Source;
use sqlx::SqlitePool;

use crate::{DbError, Listen, ListeningRecap, RecapBucket, RecapEntry, RecapPeak, RecapWindow};

const DAY: i64 = 86_400;

const IN_WINDOW: &str = "source = ?1 AND played_at >= ?2 AND played_at < ?3";

// An album without an id is told apart by its name and artist; a listen with neither has no album.
const ALBUM_GROUP: &str = "CASE WHEN album_id != '' THEN 'id:' || album_id \
     WHEN album != '' THEN 'name:' || lower(album) || char(31) || lower(artist) END";

/// Count one listen and log it, stamped with the local offset it was heard at.
pub async fn record(pool: &SqlitePool, source: &Source, listen: &Listen) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query(super::cfg_store::BUMP_LISTEN_COUNT)
        .bind(source.as_str())
        .bind(&listen.track_key)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO listens (source, track_key, played_at, utc_offset, duration_ms, \
                              title, artist, artist_key, album, album_id) \
         VALUES (?1, ?2, unixepoch(), unixepoch('now', 'localtime') - unixepoch('now'), \
                 ?3, ?4, ?5, ?6, ?7, ?8)",
    )
    .bind(source.as_str())
    .bind(&listen.track_key)
    .bind(i64::try_from(listen.duration_ms).unwrap_or(i64::MAX))
    .bind(&listen.title)
    .bind(&listen.artist)
    .bind(listen.artist_key.as_deref())
    .bind(&listen.album)
    .bind(&listen.album_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn recap(
    pool: &SqlitePool,
    source: &Source,
    window: RecapWindow,
    top: u32,
) -> Result<ListeningRecap, DbError> {
    let span = Span {
        source: source.as_str(),
        since: window.since.unwrap_or(i64::MIN),
        until: window.until,
    };

    let totals = format!(
        "SELECT COUNT(*), COALESCE(SUM(duration_ms), 0), COUNT(DISTINCT track_key), \
                COUNT(DISTINCT CASE WHEN artist != '' THEN lower(artist) END), \
                COUNT(DISTINCT {ALBUM_GROUP}) \
           FROM listens WHERE {IN_WINDOW}"
    );
    let (plays, listened_ms, tracks, artists, albums): (i64, i64, i64, i64, i64) =
        span.bind(sqlx::query_as(&totals)).fetch_one(pool).await?;

    let top_tracks = format!(
        "SELECT track_key, title, artist, plays, ms FROM ( \
           SELECT track_key, title, artist, COUNT(*) AS plays, \
                  COALESCE(SUM(duration_ms), 0) AS ms, MAX(played_at) AS last \
             FROM listens WHERE {IN_WINDOW} GROUP BY track_key) \
          ORDER BY plays DESC, ms DESC, lower(title), track_key LIMIT ?4"
    );
    // The key is looked up for the few artists kept, not every one heard.
    let top_artists = format!(
        "SELECT (SELECT l.artist_key FROM listens l \
                  WHERE l.source = ?1 AND lower(l.artist) = g.name_key \
                    AND l.artist_key IS NOT NULL AND l.artist_key != '' \
                  ORDER BY l.played_at DESC LIMIT 1), \
                g.artist, '', g.plays, g.ms FROM ( \
           SELECT lower(artist) AS name_key, artist, COUNT(*) AS plays, \
                  COALESCE(SUM(duration_ms), 0) AS ms, MAX(played_at) AS last \
             FROM listens WHERE {IN_WINDOW} AND artist != '' GROUP BY name_key \
            ORDER BY plays DESC, ms DESC, name_key LIMIT ?4) g \
          ORDER BY g.plays DESC, g.ms DESC, g.name_key"
    );
    let top_albums = format!(
        "SELECT album_id, album, artist, plays, ms FROM ( \
           SELECT {ALBUM_GROUP} AS grp, album_id, album, artist, COUNT(*) AS plays, \
                  COALESCE(SUM(duration_ms), 0) AS ms, MAX(played_at) AS last \
             FROM listens WHERE {IN_WINDOW} GROUP BY grp HAVING grp IS NOT NULL) \
          ORDER BY plays DESC, ms DESC, lower(album), grp LIMIT ?4"
    );
    let top = i64::from(top);
    let top_tracks = span.ranked(pool, &top_tracks, top).await?;
    let top_artists = span.ranked(pool, &top_artists, top).await?;
    let top_albums = span.ranked(pool, &top_albums, top).await?;

    let bucket = match window.bucket {
        RecapBucket::Hour => "strftime('%H', local, 'unixepoch')",
        RecapBucket::Day => "local / 86400",
        RecapBucket::Month => "strftime('%Y-%m', local, 'unixepoch')",
    };
    let peak = format!(
        "SELECT CAST(strftime('%Y', local, 'unixepoch') AS INTEGER), \
                CAST(strftime('%m', local, 'unixepoch') AS INTEGER), \
                CAST(strftime('%d', local, 'unixepoch') AS INTEGER), \
                CAST(strftime('%H', local, 'unixepoch') AS INTEGER), COUNT(*) AS plays \
           FROM (SELECT played_at + utc_offset AS local, duration_ms FROM listens \
                  WHERE {IN_WINDOW}) \
          GROUP BY {bucket} ORDER BY plays DESC, SUM(duration_ms) DESC, MAX(local) DESC LIMIT 1"
    );
    let peak: Option<(i64, i64, i64, i64, i64)> = span
        .bind(sqlx::query_as(&peak))
        .fetch_optional(pool)
        .await?;

    let days = format!(
        "SELECT DISTINCT (played_at + utc_offset) / {DAY} AS day \
           FROM listens WHERE {IN_WINDOW} ORDER BY day"
    );
    let days: Vec<i64> = span
        .bind_scalar(sqlx::query_scalar(&days))
        .fetch_all(pool)
        .await?;

    Ok(ListeningRecap {
        listened_ms: count(listened_ms),
        plays: count(plays),
        tracks: count(tracks),
        artists: count(artists),
        albums: count(albums),
        top_tracks,
        top_artists,
        top_albums,
        peak: peak.map(|(year, month, day, hour, plays)| RecapPeak {
            year: i32::try_from(year).unwrap_or_default(),
            month: u8::try_from(month).unwrap_or_default(),
            day: u8::try_from(day).unwrap_or_default(),
            hour: u8::try_from(hour).unwrap_or_default(),
            plays: count(plays),
        }),
        longest_streak_days: longest_streak(&days),
    })
}

/// The bind values every recap query opens with.
struct Span<'a> {
    source: &'a str,
    since: i64,
    until: i64,
}

type Args<'q> = sqlx::sqlite::SqliteArguments<'q>;

impl<'a> Span<'a> {
    fn bind<'q, O>(
        &self,
        query: sqlx::query::QueryAs<'q, sqlx::Sqlite, O, Args<'q>>,
    ) -> sqlx::query::QueryAs<'q, sqlx::Sqlite, O, Args<'q>>
    where
        'a: 'q,
    {
        query.bind(self.source).bind(self.since).bind(self.until)
    }

    fn bind_scalar<'q, O>(
        &self,
        query: sqlx::query::QueryScalar<'q, sqlx::Sqlite, O, Args<'q>>,
    ) -> sqlx::query::QueryScalar<'q, sqlx::Sqlite, O, Args<'q>>
    where
        'a: 'q,
    {
        query.bind(self.source).bind(self.since).bind(self.until)
    }

    async fn ranked(
        &self,
        pool: &SqlitePool,
        sql: &str,
        top: i64,
    ) -> Result<Vec<RecapEntry>, DbError> {
        let rows: Vec<(Option<String>, String, String, i64, i64)> = self
            .bind(sqlx::query_as(sql))
            .bind(top)
            .fetch_all(pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|(key, title, subtitle, plays, listened_ms)| RecapEntry {
                key: key.filter(|key| !key.is_empty()),
                title,
                subtitle,
                plays: count(plays),
                listened_ms: count(listened_ms),
            })
            .collect())
    }
}

fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// The longest run of consecutive days in an ascending list of distinct day numbers.
fn longest_streak(days: &[i64]) -> u32 {
    let mut best = 0;
    let mut run = 0;
    let mut previous = None;
    for &day in days {
        run = match previous {
            Some(previous) if previous + 1 == day => run + 1,
            _ => 1,
        };
        best = best.max(run);
        previous = Some(day);
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: i64 = 3600;
    // 2026-03-10 12:00 UTC.
    const NOW: i64 = 1_773_144_000;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::backend::migrations::run_migrations(&pool, None)
            .await
            .unwrap();
        pool
    }

    fn window(since: Option<i64>, bucket: RecapBucket) -> RecapWindow {
        RecapWindow {
            since,
            until: NOW + 1,
            bucket,
        }
    }

    struct Heard<'a> {
        key: &'a str,
        artist: &'a str,
        album: &'a str,
        secs: u64,
    }

    fn song<'a>(key: &'a str, artist: &'a str, album: &'a str, secs: u64) -> Heard<'a> {
        Heard {
            key,
            artist,
            album,
            secs,
        }
    }

    async fn heard(
        pool: &SqlitePool,
        source: &Source,
        song: &Heard<'_>,
        played_at: i64,
        offset: i64,
    ) {
        sqlx::query(
            "INSERT INTO listens (source, track_key, played_at, utc_offset, duration_ms, \
                                  title, artist, artist_key, album, album_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?2, ?6, ?7, ?8, ?8)",
        )
        .bind(source.as_str())
        .bind(song.key)
        .bind(played_at)
        .bind(offset)
        .bind((song.secs * 1000) as i64)
        .bind(song.artist)
        .bind(format!("key:{}", song.artist.to_lowercase()))
        .bind(song.album)
        .execute(pool)
        .await
        .unwrap();
    }

    fn ranking(entries: &[RecapEntry]) -> Vec<(&str, u64)> {
        entries
            .iter()
            .map(|entry| (entry.title.as_str(), entry.plays))
            .collect()
    }

    #[test]
    fn a_streak_is_the_longest_run_of_consecutive_days() {
        assert_eq!(longest_streak(&[]), 0);
        assert_eq!(longest_streak(&[5]), 1);
        assert_eq!(longest_streak(&[1, 2, 3, 5, 6]), 3);
        assert_eq!(longest_streak(&[1, 3, 4, 5, 6, 8, 9]), 4);
    }

    #[tokio::test]
    async fn a_recorded_listen_is_counted_and_logged() {
        let pool = mem_pool().await;
        let source = Source::default();
        let listen = Listen {
            track_key: "/a.flac".into(),
            title: "A".into(),
            artist: "Artist".into(),
            artist_key: Some("artist".into()),
            album: "Album".into(),
            album_id: "al-1".into(),
            duration_ms: 180_000,
        };
        record(&pool, &source, &listen).await.unwrap();
        record(&pool, &source, &listen).await.unwrap();

        let history = super::super::cfg_store::play_history(&pool, &source)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].count, 2);

        let all_time = RecapWindow {
            since: None,
            until: i64::MAX,
            bucket: RecapBucket::Month,
        };
        let recap = recap(&pool, &source, all_time, 5).await.unwrap();
        assert_eq!((recap.plays, recap.listened_ms), (2, 360_000));
        assert_eq!(recap.top_artists[0].key.as_deref(), Some("artist"));
        assert_eq!(recap.top_albums[0].key.as_deref(), Some("al-1"));
        assert_eq!(recap.longest_streak_days, 1);
    }

    #[tokio::test]
    async fn a_recap_sums_one_source_and_ranks_by_plays() {
        let pool = mem_pool().await;
        let source = Source::default();
        let one = song("t1", "Alpha", "First", 200);
        let two = song("t2", "Alpha", "First", 100);
        let three = song("t3", "beta", "Second", 300);
        let four = song("t4", "Beta", "", 60);
        let at = NOW - HOUR;
        for _ in 0..3 {
            heard(&pool, &source, &two, at, 0).await;
        }
        for _ in 0..2 {
            heard(&pool, &source, &one, at, 0).await;
            heard(&pool, &source, &three, at, 0).await;
        }
        // The latest listen spells the artist's name.
        heard(&pool, &source, &four, at + 60, 0).await;
        heard(&pool, &Source::Server("srv".into()), &one, at, 0).await;

        let recap = recap(&pool, &source, window(None, RecapBucket::Month), 2)
            .await
            .unwrap();

        assert_eq!(recap.plays, 8);
        assert_eq!(recap.listened_ms, (3 * 100 + 2 * 200 + 2 * 300 + 60) * 1000);
        assert_eq!((recap.tracks, recap.artists, recap.albums), (4, 2, 2));
        // Two plays each: the longer listen ranks first.
        assert_eq!(ranking(&recap.top_tracks), [("t2", 3), ("t3", 2)]);
        assert_eq!(ranking(&recap.top_artists), [("Alpha", 5), ("Beta", 3)]);
        assert_eq!(recap.top_artists[1].subtitle, "");
        assert_eq!(ranking(&recap.top_albums), [("First", 5), ("Second", 2)]);
        assert_eq!(recap.top_albums[0].subtitle, "Alpha");
        let peak = recap.peak.expect("a busiest month");
        assert_eq!((peak.year, peak.month, peak.plays), (2026, 3, 8));
    }

    #[tokio::test]
    async fn a_window_keeps_only_the_listens_since_its_start() {
        let pool = mem_pool().await;
        let source = Source::default();
        let one = song("t1", "Alpha", "First", 100);
        heard(&pool, &source, &one, NOW - 2 * HOUR, 0).await;
        heard(&pool, &source, &one, NOW - 3 * DAY, 0).await;
        heard(&pool, &source, &one, NOW - 20 * DAY, 0).await;
        heard(&pool, &source, &one, NOW - 200 * DAY, 0).await;
        heard(&pool, &source, &one, NOW - 400 * DAY, 0).await;
        // Played after the window closes.
        heard(&pool, &source, &one, NOW + HOUR, 0).await;

        let mut plays = Vec::new();
        for days in [Some(1), Some(7), Some(30), Some(365), None] {
            let since = days.map(|days| NOW - days * DAY);
            let recap = recap(&pool, &source, window(since, RecapBucket::Day), 5)
                .await
                .unwrap();
            plays.push(recap.plays);
        }
        assert_eq!(plays, [1, 2, 3, 4, 5]);

        let edge = recap(
            &pool,
            &source,
            window(Some(NOW - 3 * DAY), RecapBucket::Day),
            5,
        )
        .await
        .unwrap();
        assert_eq!(edge.plays, 2, "a listen at the window's start is in it");
    }

    #[tokio::test]
    async fn the_busiest_bucket_is_read_on_the_local_clock() {
        let pool = mem_pool().await;
        let source = Source::default();
        let one = song("t1", "Alpha", "First", 100);
        // 09:00 UTC twice, which is 11:00 two hours east.
        heard(&pool, &source, &one, NOW - 3 * HOUR, 2 * HOUR).await;
        heard(&pool, &source, &one, NOW - 3 * HOUR + 60, 2 * HOUR).await;
        heard(&pool, &source, &one, NOW - 4 * HOUR, 0).await;

        let hour = recap(
            &pool,
            &source,
            window(Some(NOW - DAY), RecapBucket::Hour),
            5,
        )
        .await
        .unwrap()
        .peak
        .expect("a busiest hour");
        assert_eq!((hour.hour, hour.plays), (11, 2));

        // 23:00 UTC on 8 March is 9 March an hour east.
        heard(&pool, &source, &one, NOW - DAY - 13 * HOUR, HOUR).await;
        heard(&pool, &source, &one, NOW - DAY - 13 * HOUR + 60, HOUR).await;
        heard(&pool, &source, &one, NOW - DAY - 13 * HOUR + 120, HOUR).await;
        heard(&pool, &source, &one, NOW - DAY - 13 * HOUR + 180, HOUR).await;
        let day = recap(
            &pool,
            &source,
            window(Some(NOW - 7 * DAY), RecapBucket::Day),
            5,
        )
        .await
        .unwrap()
        .peak
        .expect("a busiest day");
        assert_eq!((day.year, day.month, day.day, day.plays), (2026, 3, 9, 4));

        let empty = recap(
            &pool,
            &Source::Server("none".into()),
            window(None, RecapBucket::Month),
            5,
        )
        .await
        .unwrap();
        assert_eq!(
            (empty.plays, empty.peak, empty.longest_streak_days),
            (0, None, 0)
        );
        assert!(empty.top_tracks.is_empty());
    }

    #[tokio::test]
    async fn the_streak_counts_local_days_inside_the_window() {
        let pool = mem_pool().await;
        let source = Source::default();
        let one = song("t1", "Alpha", "First", 100);
        // Ten days in a row, the first six before the week began.
        for back in 0..10 {
            heard(&pool, &source, &one, NOW - HOUR - back * DAY, 0).await;
        }
        // 22:00 UTC is the next day three hours east: a second listen on the same local day.
        heard(&pool, &source, &one, NOW - DAY - 14 * HOUR, 3 * HOUR).await;

        let all = recap(&pool, &source, window(None, RecapBucket::Month), 5)
            .await
            .unwrap();
        assert_eq!(all.longest_streak_days, 10);
        let week = recap(
            &pool,
            &source,
            window(Some(NOW - 7 * DAY), RecapBucket::Day),
            5,
        )
        .await
        .unwrap();
        assert_eq!(week.longest_streak_days, 7);
    }
}

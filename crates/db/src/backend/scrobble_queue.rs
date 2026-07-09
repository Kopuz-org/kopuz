//! Offline scrobble backlog persistence (issue #335). See the `scrobble_queue`
//! migration; the retry orchestration (submit, give-up per service, drain)
//! lives in `kopuz-scrobble`, which calls these through the `Storage` trait.

use sqlx::SqlitePool;

use crate::{DbError, QueuedScrobbleRow, Service};

/// Newest listens kept; when the backlog overflows the oldest are dropped. A
/// "listen" is one `listened_at` group, so a listen owed to three services
/// still counts as one toward the cap.
const MAX_QUEUED_LISTENS: i64 = 500;

/// A raw `scrobble_queue` row as selected: `(listened_at, artist, title, album,
/// service, listen_info)`.
type QueueRow = (i64, String, String, Option<String>, String, Option<String>);

pub async fn all(pool: &SqlitePool) -> Result<Vec<QueuedScrobbleRow>, DbError> {
    let rows: Vec<QueueRow> = sqlx::query_as(
        "SELECT listened_at, artist, title, album, service, listen_info \
             FROM scrobble_queue ORDER BY listened_at ASC, id ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(
            |(listened_at, artist, title, album, service, listen_info)| {
                Some(QueuedScrobbleRow {
                    listened_at,
                    artist,
                    title,
                    album,
                    service: Service::from_tag(&service)?,
                    listen_info,
                })
            },
        )
        .collect())
}

pub async fn push(pool: &SqlitePool, row: &QueuedScrobbleRow) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO scrobble_queue \
             (listened_at, artist, title, album, service, listen_info) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(listened_at, artist, title, service) DO UPDATE SET \
             album = COALESCE(excluded.album, album), \
             listen_info = COALESCE(excluded.listen_info, listen_info)",
    )
    .bind(row.listened_at)
    .bind(&row.artist)
    .bind(&row.title)
    .bind(&row.album)
    .bind(row.service.as_tag())
    .bind(&row.listen_info)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "DELETE FROM scrobble_queue WHERE listened_at NOT IN \
         (SELECT listened_at FROM scrobble_queue \
          GROUP BY listened_at ORDER BY listened_at DESC LIMIT ?1)",
    )
    .bind(MAX_QUEUED_LISTENS)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn delete(
    pool: &SqlitePool,
    listened_at: i64,
    artist: &str,
    title: &str,
    service: Service,
) -> Result<(), DbError> {
    sqlx::query(
        "DELETE FROM scrobble_queue \
         WHERE listened_at = ?1 AND artist = ?2 AND title = ?3 AND service = ?4",
    )
    .bind(listened_at)
    .bind(artist)
    .bind(title)
    .bind(service.as_tag())
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::QueuedScrobbleRow;

    async fn mem_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::backend::migrations::run_migrations(&pool)
            .await
            .unwrap();
        pool
    }

    fn row(title: &str, ts: i64, service: Service) -> QueuedScrobbleRow {
        QueuedScrobbleRow {
            listened_at: ts,
            artist: "Artist".into(),
            title: title.into(),
            album: None,
            service,
            listen_info: None,
        }
    }

    #[tokio::test]
    async fn push_caps_backlog_and_drops_oldest_listens() {
        let pool = mem_pool().await;
        for i in 0..(MAX_QUEUED_LISTENS + 10) {
            push(&pool, &row(&format!("t{i}"), i, Service::LastFm))
                .await
                .unwrap();
        }
        let all = all(&pool).await.unwrap();
        assert_eq!(all.len() as i64, MAX_QUEUED_LISTENS);
        assert_eq!(all.first().unwrap().listened_at, 10);
    }

    #[tokio::test]
    async fn push_merges_same_listen_and_delete_is_per_service() {
        let pool = mem_pool().await;
        push(&pool, &row("Song", 42, Service::LastFm))
            .await
            .unwrap();
        let mut lb = row("Song", 42, Service::ListenBrainz);
        lb.listen_info = Some(r#"{"duration_ms":180000}"#.into());
        push(&pool, &lb).await.unwrap();
        push(&pool, &row("Song", 42, Service::LastFm))
            .await
            .unwrap();

        let all_rows = all(&pool).await.unwrap();
        assert_eq!(all_rows.len(), 2);

        delete(&pool, 42, "Artist", "Song", Service::LastFm)
            .await
            .unwrap();
        let rest = all(&pool).await.unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].service, Service::ListenBrainz);
        assert_eq!(
            rest[0].listen_info.as_deref(),
            Some(r#"{"duration_ms":180000}"#)
        );
    }
}

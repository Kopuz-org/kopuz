//! A frontend's preferences: opaque strings the daemon stores and never reads.

use sqlx::SqlitePool;

use crate::DbError;

pub async fn all(pool: &SqlitePool, frontend: &str) -> Result<Vec<(String, String)>, DbError> {
    Ok(
        sqlx::query_as("SELECT key, value FROM frontend_prefs WHERE frontend = ?1 ORDER BY key")
            .bind(frontend)
            .fetch_all(pool)
            .await?,
    )
}

/// Store each entry the frontend does not hold yet, leaving what it stored itself.
pub(crate) async fn seed(
    conn: &mut sqlx::SqliteConnection,
    frontend: &str,
    entries: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<(), DbError> {
    for (key, value) in entries {
        sqlx::query(
            "INSERT OR IGNORE INTO frontend_prefs (frontend, key, value) VALUES (?1, ?2, ?3)",
        )
        .bind(frontend)
        .bind(key)
        .bind(value.to_string())
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// Apply every entry or none; answers the keys whose stored value changed, or `None` when the result would pass `budget` bytes.
pub async fn set(
    pool: &SqlitePool,
    frontend: &str,
    entries: &[(String, Option<String>)],
    budget: usize,
) -> Result<Option<Vec<String>>, DbError> {
    let mut tx = super::begin_immediate(pool).await?;
    let mut changed = Vec::new();
    for (key, value) in entries {
        let stored: Option<String> =
            sqlx::query_scalar("SELECT value FROM frontend_prefs WHERE frontend = ?1 AND key = ?2")
                .bind(frontend)
                .bind(key)
                .fetch_optional(&mut *tx)
                .await?;
        match value {
            Some(value) if stored.as_ref() != Some(value) => {
                sqlx::query(
                    "INSERT INTO frontend_prefs (frontend, key, value) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(frontend, key) DO UPDATE SET value = ?3",
                )
                .bind(frontend)
                .bind(key)
                .bind(value)
                .execute(&mut *tx)
                .await?;
                changed.push(key.clone());
            }
            None if stored.is_some() => {
                sqlx::query("DELETE FROM frontend_prefs WHERE frontend = ?1 AND key = ?2")
                    .bind(frontend)
                    .bind(key)
                    .execute(&mut *tx)
                    .await?;
                changed.push(key.clone());
            }
            _ => {}
        }
    }
    let held: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(LENGTH(CAST(key AS BLOB)) + LENGTH(CAST(value AS BLOB))), 0) \
         FROM frontend_prefs WHERE frontend = ?1",
    )
    .bind(frontend)
    .fetch_one(&mut *tx)
    .await?;
    if usize::try_from(held).unwrap_or(usize::MAX) > budget {
        // Dropping the transaction uncommitted is its rollback.
        drop(tx);
        return Ok(None);
    }
    tx.commit().await?;
    Ok(Some(changed))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOM: usize = usize::MAX;

    async fn pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = super::super::open_pool(&dir.path().join("t.db"))
            .await
            .expect("pool");
        super::super::migrations::run_migrations(&pool, None)
            .await
            .expect("migrate");
        (dir, pool)
    }

    fn put(key: &str, value: &str) -> (String, Option<String>) {
        (key.to_string(), Some(value.to_string()))
    }

    fn pairs(rows: &[(&str, &str)]) -> Vec<(String, String)> {
        rows.iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn prefs_are_namespaced_by_frontend() {
        let (_dir, pool) = pool().await;
        set(&pool, "gpui", &[put("skin", "dark")], ROOM)
            .await
            .unwrap();
        set(&pool, "dioxus", &[put("skin", "light")], ROOM)
            .await
            .unwrap();

        assert_eq!(
            all(&pool, "gpui").await.unwrap(),
            pairs(&[("skin", "dark")])
        );
        assert_eq!(
            all(&pool, "dioxus").await.unwrap(),
            pairs(&[("skin", "light")])
        );
        assert!(all(&pool, "nobody").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn only_keys_that_changed_are_reported() {
        let (_dir, pool) = pool().await;
        let first = set(&pool, "gpui", &[put("a", "1"), put("b", "2")], ROOM)
            .await
            .unwrap();
        assert_eq!(first, Some(vec!["a".to_string(), "b".to_string()]));

        let second = set(
            &pool,
            "gpui",
            &[
                put("a", "1"),
                put("b", "3"),
                ("missing".to_string(), None),
                ("a".to_string(), None),
            ],
            ROOM,
        )
        .await
        .unwrap();
        assert_eq!(second, Some(vec!["b".to_string(), "a".to_string()]));
        assert_eq!(all(&pool, "gpui").await.unwrap(), pairs(&[("b", "3")]));
    }

    #[tokio::test]
    async fn a_value_is_stored_verbatim() {
        let (_dir, pool) = pool().await;
        let value = "{\"not\": \"parsed\"}\n\u{1f3b5} \0 tail";
        set(&pool, "gpui", &[put("blob", value), put("empty", "")], ROOM)
            .await
            .unwrap();
        assert_eq!(
            all(&pool, "gpui").await.unwrap(),
            pairs(&[("blob", value), ("empty", "")])
        );
    }

    #[tokio::test]
    async fn a_refused_row_leaves_the_whole_write_undone() {
        let (_dir, pool) = pool().await;
        set(&pool, "gpui", &[put("keep", "1")], ROOM).await.unwrap();
        let refused = set(&pool, "gpui", &[put("keep", "2"), put("", "x")], ROOM).await;
        assert!(refused.is_err(), "the table refuses an empty key");
        assert_eq!(all(&pool, "gpui").await.unwrap(), pairs(&[("keep", "1")]));
    }

    #[tokio::test]
    async fn a_write_past_the_budget_is_undone_and_others_do_not_count() {
        let (_dir, pool) = pool().await;
        set(&pool, "dioxus", &[put("big", &"x".repeat(64))], ROOM)
            .await
            .unwrap();
        assert!(
            set(&pool, "gpui", &[put("a", "123456")], 8)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            set(&pool, "gpui", &[put("b", "123")], 8)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(all(&pool, "gpui").await.unwrap(), pairs(&[("a", "123456")]));
        assert!(
            set(&pool, "gpui", &[("a".to_string(), None), put("b", "1")], 8)
                .await
                .unwrap()
                .is_some()
        );
    }
}

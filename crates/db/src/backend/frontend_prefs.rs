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

/// Apply every entry or none; answers the keys whose stored value actually changed.
pub async fn set(
    pool: &SqlitePool,
    frontend: &str,
    entries: &[(String, Option<String>)],
) -> Result<Vec<String>, DbError> {
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
    tx.commit().await?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

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
        set(&pool, "gpui", &[put("skin", "dark")]).await.unwrap();
        set(&pool, "dioxus", &[put("skin", "light")]).await.unwrap();

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
        let first = set(&pool, "gpui", &[put("a", "1"), put("b", "2")])
            .await
            .unwrap();
        assert_eq!(first, vec!["a", "b"]);

        let second = set(
            &pool,
            "gpui",
            &[
                put("a", "1"),
                put("b", "3"),
                ("missing".to_string(), None),
                ("a".to_string(), None),
            ],
        )
        .await
        .unwrap();
        assert_eq!(second, vec!["b", "a"]);
        assert_eq!(all(&pool, "gpui").await.unwrap(), pairs(&[("b", "3")]));
    }

    #[tokio::test]
    async fn a_value_is_stored_verbatim() {
        let (_dir, pool) = pool().await;
        let value = "{\"not\": \"parsed\"}\n\u{1f3b5} \0 tail";
        set(&pool, "gpui", &[put("blob", value), put("empty", "")])
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
        set(&pool, "gpui", &[put("keep", "1")]).await.unwrap();
        let refused = set(&pool, "gpui", &[put("keep", "2"), put("", "x")]).await;
        assert!(refused.is_err(), "the table refuses an empty key");
        assert_eq!(all(&pool, "gpui").await.unwrap(), pairs(&[("keep", "1")]));
    }
}

//! The September credit schema used migration numbers later reused by the artist
//! schema. Keep its original SQL and ledger intact, then bridge with new migrations
//! before the shared queue migrations need artist rows. Unicode name folding runs
//! between creating those rows and dropping the old IDs, just like the other fills.

use sqlx::migrate::{MigrateError, Migrator};
use sqlx::{SqliteConnection, SqlitePool};

use super::{ARTISTS_CREATED, MIGRATOR, applied, checksum_matches, file_unlinked};
use crate::DbError;

static LEGACY: Migrator = sqlx::migrate!("./legacy-migrations");

#[cfg(test)]
mod tests;

fn migrator() -> Migrator {
    let mut migrations: Vec<_> = MIGRATOR
        .iter()
        .filter(|m| !LEGACY.iter().any(|old| old.version == m.version))
        .chain(LEGACY.iter())
        .cloned()
        .collect();
    migrations.sort_by_key(|m| m.version);
    Migrator {
        migrations: migrations.into(),
        ..Migrator::DEFAULT
    }
}

pub(super) async fn for_database(pool: &SqlitePool) -> Result<Option<Migrator>, DbError> {
    if !applied(pool, ARTISTS_CREATED).await? {
        return Ok(None);
    }
    let stored: Vec<(i64, Vec<u8>, bool)> =
        sqlx::query_as("SELECT version, checksum, success FROM _sqlx_migrations")
            .fetch_all(pool)
            .await?;
    let legacy = stored.iter().any(|(version, checksum, _)| {
        *version == ARTISTS_CREATED
            && LEGACY
                .iter()
                .any(|m| m.version == *version && checksum_matches(m, checksum))
    });
    if !legacy {
        return Ok(None);
    }

    let migrator = migrator();
    for (version, checksum, success) in stored {
        if !success {
            return Err(MigrateError::Dirty(version).into());
        }
        let migration = migrator
            .iter()
            .find(|m| m.version == version)
            .ok_or(MigrateError::VersionMissing(version))?;
        if !checksum_matches(migration, &checksum) {
            return Err(MigrateError::VersionMismatch(version).into());
        }
    }
    Ok(Some(migrator))
}

pub(super) async fn fill(pool: &SqlitePool) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    for sql in [
        "DELETE FROM track_credits",
        "UPDATE albums SET artist_pk = NULL",
        "DELETE FROM artists",
    ] {
        sqlx::query(sql).execute(&mut *tx).await?;
    }

    let credits: Vec<(i64, i64, String, String, Option<String>)> = sqlx::query_as(
        "SELECT c.track_pk, c.position, t.source, c.name, c.artist_id \
           FROM legacy_track_credits c JOIN tracks t ON t.rowid_pk = c.track_pk \
          ORDER BY c.track_pk, c.position",
    )
    .fetch_all(&mut *tx)
    .await?;
    for (track, position, source, name, id) in credits {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let artist = file_artist(&mut tx, &source, name, id.as_deref()).await?;
        sqlx::query(
            "INSERT INTO track_credits (track_pk, position, artist_pk, name) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(track)
        .bind(position)
        .bind(artist)
        .bind(name)
        .execute(&mut *tx)
        .await?;
    }

    let albums: Vec<(i64, String, String, Option<String>)> =
        sqlx::query_as("SELECT rowid_pk, source, artist, artist_id FROM albums ORDER BY rowid_pk")
            .fetch_all(&mut *tx)
            .await?;
    for (album, source, name, id) in albums {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let artist = file_artist(&mut tx, &source, name, id.as_deref()).await?;
        sqlx::query("UPDATE albums SET artist_pk = ?1 WHERE rowid_pk = ?2")
            .bind(artist)
            .bind(album)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Uses the schema at this migration, independent of later changes to the live writer.
async fn file_artist(
    conn: &mut SqliteConnection,
    source: &str,
    name: &str,
    id: Option<&str>,
) -> Result<i64, DbError> {
    let Some(id) = id.map(str::trim).filter(|id| !id.is_empty()) else {
        return file_unlinked(conn, source, name).await;
    };
    Ok(sqlx::query_scalar(
        "INSERT INTO artists (source, source_artist_id, name, name_key) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(source, source_artist_id) DO UPDATE SET name = artists.name RETURNING id",
    )
    .bind(source)
    .bind(id)
    .bind(name)
    .bind(utils::artist::normalize_artist_key(name))
    .fetch_one(conn)
    .await?)
}

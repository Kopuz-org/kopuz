//! The artist key a frontend holds: minted here and read only here.

use api::{ApiError, ArtistKey as WireKey};
use config::Source;

const ISSUED: &str = "src";
const LIBRARY: &str = "lib";

/// Who a key names: the artist a source issued an id for, or a library row for one it issued none for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Artist {
    Issued { source: Source, id: String },
    Library(i64),
}

/// Escapes only `%` and `:`, so the parts split back apart whatever a source id holds.
fn escape(part: &str) -> String {
    part.replace('%', "%25").replace(':', "%3A")
}

fn unescape(part: &str) -> Option<String> {
    percent_encoding::percent_decode_str(part)
        .decode_utf8()
        .ok()
        .map(|part| part.into_owned())
}

/// The artist `source` issued `id` for, whether or not the library holds it.
pub(crate) fn issued(source: &Source, id: &str) -> WireKey {
    WireKey::new(format!(
        "{ISSUED}:{}:{}",
        escape(source.as_str()),
        escape(id)
    ))
}

/// A library artist row whose source issued it no id.
pub(crate) fn library(pk: i64) -> WireKey {
    WireKey::new(format!("{LIBRARY}:{pk}"))
}

/// A listed artist of `source`: by its id where the source issued one, so a catalog can open it too.
pub(crate) fn of_row(source: &Source, row: &db::ArtistRow) -> WireKey {
    match &row.source_id {
        Some(id) => issued(source, id),
        None => library(row.pk),
    }
}

/// The artist a credit names: by the id its listing source issued, else by the library row it is filed under.
pub(crate) fn of_credit(credit: &reader::ArtistCredit) -> Option<WireKey> {
    match (&credit.source, &credit.id, credit.artist_pk) {
        (Some(source), Some(id), _) => Some(issued(source, id)),
        (_, _, Some(pk)) => Some(library(pk)),
        _ => None,
    }
}

pub(crate) fn read(key: &WireKey) -> Result<Artist, ApiError> {
    let invalid = || ApiError::invalid_input("not an artist key");
    let mut parts = key.as_str().split(':');
    let artist = match (parts.next(), parts.next(), parts.next()) {
        (Some(ISSUED), Some(source), Some(id)) => Artist::Issued {
            source: Source::from_column(&unescape(source).ok_or_else(invalid)?),
            id: unescape(id).ok_or_else(invalid)?,
        },
        (Some(LIBRARY), Some(pk), None) => Artist::Library(pk.parse().map_err(|_| invalid())?),
        _ => return Err(invalid()),
    };
    match parts.next() {
        Some(_) => Err(invalid()),
        None => Ok(artist),
    }
}

/// The library row a key names in `source`; a key minted under another source names nothing here.
pub(crate) async fn row(
    db: &db::Db,
    source: &Source,
    key: &WireKey,
) -> Result<db::ArtistRow, ApiError> {
    let db_error = |error: db::DbError| ApiError::internal(format!("database error: {error}"));
    let missing = || ApiError::not_found("the library files no such artist");
    let pk = match read(key)? {
        Artist::Issued { source: issuer, .. } if issuer != *source => {
            return Err(ApiError::not_found("that artist belongs to another source"));
        }
        Artist::Issued { id, .. } => db
            .artist_pk(source, &id)
            .await
            .map_err(db_error)?
            .ok_or_else(missing)?,
        Artist::Library(pk) => pk,
    };
    db.artist(source, pk)
        .await
        .map_err(db_error)?
        .ok_or_else(missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_reads_back_as_the_artist_it_was_minted_for() {
        for source in [
            Source::Local,
            Source::Server("3c696f0a".into()),
            Source::LocalLibrary("local:/music:odd%dir".into()),
        ] {
            let artist = Artist::Issued {
                source: source.clone(),
                id: "UC-a:b%c".into(),
            };
            assert_eq!(read(&issued(&source, "UC-a:b%c")).unwrap(), artist);
        }
        assert_eq!(read(&library(42)).unwrap(), Artist::Library(42));
    }

    #[test]
    fn a_credit_is_keyed_by_the_source_that_listed_it_and_nothing_else() {
        let listed =
            |source: Option<&str>, id: Option<&str>, pk: Option<i64>| reader::ArtistCredit {
                name: "Ada".into(),
                id: id.map(Into::into),
                source: source.map(|source| Source::Server(source.into())),
                artist_pk: pk,
            };
        let elsewhere = Source::Server("srv-a".into());

        assert_eq!(
            of_credit(&listed(Some("srv-a"), Some("ar-1"), Some(7))),
            Some(issued(&elsewhere, "ar-1"))
        );
        assert_eq!(
            of_credit(&listed(Some("srv-a"), None, Some(7))),
            Some(library(7))
        );
        assert_eq!(
            of_credit(&listed(None, Some("ar-1"), None)),
            None,
            "an id from nowhere"
        );
        assert_eq!(
            of_credit(&listed(Some("srv-a"), None, None)),
            None,
            "a bare name"
        );
    }

    #[test]
    fn a_string_no_daemon_minted_is_refused() {
        for junk in [
            "",
            "ar-12",
            "src:srv",
            "src:a:b:c",
            "who:srv:x",
            "src:srv:%ff",
            "lib:x",
            "lib:1:2",
            "name:srv:ada",
        ] {
            let error = read(&WireKey::new(junk)).unwrap_err();
            assert_eq!(error.code, api::ErrorCode::InvalidInput, "{junk}");
        }
    }
}

//! The artist key a frontend holds: minted here and read only here.

use api::{ApiError, ArtistKey as WireKey};
use config::Source;
use utils::artist::ArtistKey;

const ID: &str = "id";
const NAME: &str = "name";

/// Escapes only `%` and `:`, so the three parts split back apart whatever a source id or name holds.
fn escape(part: &str) -> String {
    part.replace('%', "%25").replace(':', "%3A")
}

fn unescape(part: &str) -> Option<String> {
    percent_encoding::percent_decode_str(part)
        .decode_utf8()
        .ok()
        .map(|part| part.into_owned())
}

/// The key naming `artist` within `source`.
pub(crate) fn mint(source: &Source, artist: &ArtistKey) -> WireKey {
    let (tag, value) = match artist {
        ArtistKey::Id(id) => (ID, id),
        ArtistKey::Name(name) => (NAME, name),
    };
    WireKey::new(format!(
        "{tag}:{}:{}",
        escape(source.as_str()),
        escape(value)
    ))
}

/// The key a row credits under `source`: its source's id where it has one, else its name.
pub(crate) fn of(source: &Source, name: &str, id: Option<&str>) -> WireKey {
    mint(source, &ArtistKey::of(name, id))
}

fn read(key: &WireKey) -> Option<(Source, ArtistKey)> {
    let mut parts = key.as_str().split(':');
    let (tag, source, value) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let source = Source::from_column(&unescape(source)?);
    let value = unescape(value)?;
    match tag {
        ID => Some((source, ArtistKey::Id(value))),
        NAME => Some((source, ArtistKey::Name(value))),
        _ => None,
    }
}

/// The artist a key names, which must be one of `active`'s: a key from another source is stale.
pub(crate) fn within(key: &WireKey, active: &Source) -> Result<ArtistKey, ApiError> {
    match read(key) {
        Some((source, artist)) if source == *active => Ok(artist),
        Some(_) => Err(ApiError::not_found("that artist belongs to another source")),
        None => Err(ApiError::invalid_input("not an artist key")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_reads_back_as_the_artist_it_was_minted_for() {
        let sources = [
            Source::Local,
            Source::Server("3c696f0a".into()),
            Source::LocalLibrary("local:/music:odd%dir".into()),
        ];
        let artists = [
            ArtistKey::Id("UC-a:b%c".into()),
            ArtistKey::of("Cartoon, Jéja: live", None),
        ];
        for source in &sources {
            for artist in &artists {
                assert_eq!(within(&mint(source, artist), source).unwrap(), *artist);
            }
        }
    }

    #[test]
    fn a_key_from_another_source_opens_nothing() {
        let key = mint(&Source::Server("a".into()), &ArtistKey::Id("ar-12".into()));

        let error = within(&key, &Source::Server("b".into())).unwrap_err();

        assert_eq!(error.code, api::ErrorCode::NotFound);
    }

    #[test]
    fn a_string_no_daemon_minted_is_refused() {
        for junk in ["", "ar-12", "id:srv", "id:a:b:c", "who:srv:x", "id:srv:%ff"] {
            let error = within(&WireKey::new(junk), &Source::Server("srv".into())).unwrap_err();
            assert_eq!(error.code, api::ErrorCode::InvalidInput, "{junk}");
        }
    }
}

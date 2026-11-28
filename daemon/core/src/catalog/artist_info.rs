//! An artist's biography, picture and similar artists from the active source.

use std::collections::HashMap;

use api::{ApiError, ArtistInfoView, SimilarArtist};

use super::CatalogService;
use crate::sources::source_error;

/// What a picture is remembered under until the artwork service asks for it.
fn image_id(artist_key: &str) -> String {
    format!("artist-info:{artist_key}")
}

/// A similar artist links to the library only when the library holds it, by the id the source issued.
fn similar_artists(
    similar: Vec<reader::ArtistCredit>,
    linked: &HashMap<String, String>,
) -> Vec<SimilarArtist> {
    similar
        .into_iter()
        .map(|credit| SimilarArtist {
            key: credit.id.and_then(|id| linked.get(&id).cloned()),
            name: credit.name,
        })
        .collect()
}

/// The URL of a stored picture reference; a file or a source cover id is not one the client may be shown.
fn image_url(stored: &str) -> Option<String> {
    match reader::CoverRef::parse(stored) {
        reader::CoverRef::EmbeddedUrl(url) => Some(url),
        _ => None,
    }
}

impl CatalogService {
    /// `None` when the source offers no artist info, the artist is not one it issued, or it knows nothing of them.
    pub async fn artist_info(&self, artist: &str) -> Result<Option<ArtistInfoView>, ApiError> {
        if self.library.offline_reads() {
            return Ok(None);
        }
        let source = self.source()?;
        if !source.capabilities().artist_info {
            return Ok(None);
        }
        let row = self.library.artist_row(artist).await?;
        let Some(source_id) = row.source_id.as_deref() else {
            return Ok(None);
        };

        let Some(info) = source
            .fetch_artist_info(source_id)
            .await
            .map_err(source_error)?
        else {
            return Ok(None);
        };
        let linked = self
            .db
            .linked_artist_keys(source.source())
            .await
            .map_err(super::db_error)?;
        let image = info
            .image
            .as_deref()
            .and_then(image_url)
            .and_then(|url| self.remember_thumbnail(&image_id(&row.key), Some(&url)));
        Ok(Some(ArtistInfoView {
            biography: info.biography,
            image,
            similar: similar_artists(info.similar, &linked),
            link: info.link,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_artists_the_library_holds_get_a_key() {
        let linked = HashMap::from([("ar-2".to_string(), "ar-2".to_string())]);
        let similar = similar_artists(
            vec![
                reader::ArtistCredit::linked("Boris", "ar-2"),
                reader::ArtistCredit::linked("Elsewhere", "ar-9"),
                reader::ArtistCredit::unlinked("Nobody"),
            ],
            &linked,
        );
        let keys: Vec<_> = similar.iter().map(|artist| artist.key.as_deref()).collect();
        assert_eq!(keys, [Some("ar-2"), None, None]);
        assert_eq!(similar[1].name, "Elsewhere");
    }

    #[test]
    fn only_a_url_is_a_picture_to_serve() {
        assert_eq!(
            image_url("directurl:https://img.test/a.jpg").as_deref(),
            Some("https://img.test/a.jpg")
        );
        assert_eq!(image_url("/home/me/a.jpg"), None);
        assert_eq!(image_url("none"), None);
    }
}

#![allow(non_snake_case)]

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Artwork {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlayParams {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    #[serde(rename = "catalogId")]
    pub catalog_id: Option<String>,
    #[serde(default)]
    #[serde(rename = "isLibrary")]
    pub is_library: Option<bool>,
    #[serde(default)]
    #[serde(rename = "reportingId")]
    pub reporting_id: Option<String>,
}

// ── Catalog types (used by search, get_song, get_album, etc.) ──────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArtistRef {
    pub id: String,
    #[serde(default)]
    pub attributes: Option<ArtistRefAttributes>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArtistRefAttributes {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlbumRef {
    pub id: String,
    #[serde(default)]
    pub attributes: Option<AlbumRefAttributes>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlbumRefAttributes {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist_name: String,
    #[serde(default)]
    pub artwork: Option<Artwork>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TrackData {
    pub id: String,
    #[serde(default)]
    #[serde(rename = "type")]
    pub track_type: String,
    #[serde(default)]
    pub attributes: TrackAttributes,
    #[serde(default)]
    pub relationships: TrackRelationships,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TrackAttributes {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "artistName")]
    pub artist_name: String,
    #[serde(default, rename = "albumName")]
    pub album_name: String,
    #[serde(default)]
    pub artwork: Artwork,
    #[serde(default)]
    pub durationInMillis: u64,
    #[serde(default)]
    pub trackNumber: u32,
    #[serde(default)]
    pub discNumber: u32,
    #[serde(default)]
    pub genreNames: Vec<String>,
    #[serde(default)]
    pub releaseDate: String,
    #[serde(default)]
    pub isrc: String,
    #[serde(default)]
    pub audioTraits: Vec<String>,
    #[serde(default)]
    pub contentRating: String,
    #[serde(default)]
    pub playParams: Option<PlayParams>,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TrackRelationships {
    #[serde(default)]
    pub artists: RelationshipData<Vec<ArtistRef>>,
    #[serde(default)]
    pub albums: RelationshipData<Vec<AlbumRef>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RelationshipData<T> {
    #[serde(default)]
    pub data: T,
    #[serde(default)]
    pub next: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TrackResp {
    #[serde(default)]
    pub href: String,
    #[serde(default)]
    pub next: String,
    #[serde(default)]
    pub data: Vec<TrackData>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SongResp {
    #[serde(default)]
    pub href: String,
    #[serde(default)]
    pub next: String,
    #[serde(default)]
    pub data: Vec<TrackData>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlbumData {
    pub id: String,
    #[serde(default)]
    #[serde(rename = "type")]
    pub album_type: String,
    #[serde(default)]
    pub attributes: AlbumAttributes,
    #[serde(default)]
    pub relationships: AlbumRelationships,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlbumAttributes {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artist_name: String,
    #[serde(default)]
    pub artwork: Artwork,
    #[serde(default)]
    pub genreNames: Vec<String>,
    #[serde(default)]
    pub trackCount: u32,
    #[serde(default)]
    pub releaseDate: String,
    #[serde(default)]
    pub copyright: String,
    #[serde(default)]
    pub upc: String,
    #[serde(default)]
    pub isSingle: bool,
    #[serde(default)]
    pub isCompilation: bool,
    #[serde(default)]
    pub playParams: Option<PlayParams>,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlbumRelationships {
    #[serde(default)]
    pub artists: RelationshipData<Vec<ArtistRef>>,
    #[serde(default)]
    pub tracks: TrackResp,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AlbumResp {
    #[serde(default)]
    pub href: String,
    #[serde(default)]
    pub next: String,
    #[serde(default)]
    pub data: Vec<AlbumData>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlaylistData {
    pub id: String,
    #[serde(default)]
    #[serde(rename = "type")]
    pub playlist_type: String,
    #[serde(default)]
    pub attributes: PlaylistAttributes,
    #[serde(default)]
    pub relationships: AlbumRelationships,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlaylistAttributes {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artwork: Option<Artwork>,
    #[serde(default)]
    pub description: Option<PlaylistDescription>,
    #[serde(default)]
    pub trackCount: u32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlaylistDescription {
    #[serde(default)]
    pub short: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PlaylistResp {
    #[serde(default)]
    pub href: String,
    #[serde(default)]
    pub next: String,
    #[serde(default)]
    pub data: Vec<PlaylistData>,
}

// ── Search types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchResp {
    #[serde(default)]
    pub results: SearchResults,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchResults {
    #[serde(default)]
    pub songs: Option<SearchResultList<TrackData>>,
    #[serde(default)]
    pub albums: Option<SearchResultList<AlbumData>>,
    #[serde(default)]
    pub artists: Option<SearchResultList<ArtistSearchData>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SearchResultList<T> {
    #[serde(default)]
    pub href: String,
    #[serde(default)]
    pub next: String,
    #[serde(default)]
    pub data: Vec<T>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArtistSearchData {
    pub id: String,
    #[serde(default)]
    pub attributes: ArtistSearchAttributes,
}

#[allow(non_snake_case)]
#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArtistSearchAttributes {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub genreNames: Vec<String>,
}

// ── Library types (format[resources]=map) ──────────────────────────

/// A reference entry in the `data` array of a library response.
#[derive(Debug, Clone, Deserialize)]
pub struct LibraryRef {
    pub id: String,
    #[serde(rename = "type")]
    pub ref_type: String,
}

/// The top-level library response with `data` (references) and `resources` (nested map).
/// Resources are keyed by type name, then by id: `resources["library-albums"]["l.xxx"]`.
/// We use `serde_json::Value` because the outer map may contain multiple types
/// (e.g. "artists" + "library-albums") and only one type is our target.
#[derive(Debug, Clone, Deserialize)]
pub struct LibraryResourceResponse {
    #[serde(default)]
    pub next: String,
    #[serde(default)]
    pub data: Vec<LibraryRef>,
    #[serde(default)]
    pub resources: serde_json::Value,
}

// ── Library song resource ──────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibrarySongResource {
    pub id: String,
    #[serde(rename = "type")]
    pub resource_type: String,
    #[serde(default)]
    pub attributes: LibrarySongAttributes,
    #[serde(default)]
    pub relationships: LibrarySongRelationships,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibrarySongAttributes {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artistName: String,
    #[serde(default)]
    pub albumName: String,
    #[serde(default)]
    pub artwork: Option<Artwork>,
    #[serde(default)]
    pub durationInMillis: u64,
    #[serde(default)]
    pub trackNumber: u32,
    #[serde(default)]
    pub discNumber: u32,
    #[serde(default)]
    pub genreNames: Vec<String>,
    #[serde(default)]
    pub releaseDate: String,
    #[serde(default)]
    pub contentRating: String,
    #[serde(default)]
    pub playParams: Option<PlayParams>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibrarySongRelationships {
    #[serde(default)]
    pub catalog: RelationshipData<Vec<LibraryCatalogRef>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryCatalogRef {
    pub id: String,
    #[serde(rename = "type")]
    pub ref_type: String,
}

// ── Library album resource ─────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryAlbumResource {
    pub id: String,
    #[serde(rename = "type")]
    pub resource_type: String,
    #[serde(default)]
    pub attributes: LibraryAlbumAttributes,
    #[serde(default)]
    pub relationships: LibraryAlbumRelationships,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryAlbumAttributes {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artistName: String,
    #[serde(default)]
    pub artwork: Option<Artwork>,
    #[serde(default)]
    pub trackCount: u32,
    #[serde(default)]
    pub genreNames: Vec<String>,
    #[serde(default)]
    pub releaseDate: String,
    #[serde(default)]
    pub playParams: Option<PlayParams>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryAlbumRelationships {
    #[serde(default)]
    pub artists: RelationshipData<Vec<LibraryCatalogRef>>,
}

// ── Library artist resource ────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryArtistResource {
    pub id: String,
    #[serde(rename = "type")]
    pub resource_type: String,
    #[serde(default)]
    pub attributes: LibraryArtistAttributes,
    #[serde(default)]
    pub relationships: LibraryArtistRelationships,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryArtistAttributes {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artwork: Option<Artwork>,
    #[serde(default)]
    pub genreNames: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryArtistRelationships {
    #[serde(default)]
    pub albums: RelationshipData<Vec<LibraryCatalogRef>>,
}

// ── Library playlist resource ──────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryPlaylistResource {
    pub id: String,
    #[serde(rename = "type")]
    pub resource_type: String,
    #[serde(default)]
    pub attributes: LibraryPlaylistAttributes,
    #[serde(default)]
    pub relationships: LibraryPlaylistRelationships,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryPlaylistAttributes {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub artwork: Option<Artwork>,
    #[serde(default)]
    pub trackCount: u32,
    #[serde(default)]
    pub canEdit: bool,
    #[serde(default)]
    pub canDelete: bool,
    #[serde(default)]
    #[serde(rename = "isPublic")]
    pub is_public: Option<bool>,
    #[serde(default)]
    #[serde(rename = "dateAdded")]
    pub date_added: Option<String>,
    #[serde(default)]
    pub description: Option<LibraryPlaylistDescription>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryPlaylistDescription {
    #[serde(default)]
    pub standard: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LibraryPlaylistRelationships {
    #[serde(default)]
    pub catalog: RelationshipData<Vec<LibraryCatalogRef>>,
}

// ── Library playlist tracks response (also uses resources map) ─────

#[derive(Debug, Clone, Deserialize)]
pub struct LibraryPlaylistTracksResponse {
    #[serde(default)]
    pub next: String,
    #[serde(default)]
    pub data: Vec<LibraryRef>,
    #[serde(default)]
    pub resources: serde_json::Value,
}

// ── Web playback types ─────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WebPlaybackResp {
    #[serde(default)]
    pub songList: Vec<WebPlaybackSong>,
    #[serde(default)]
    pub status: i32,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WebPlaybackSong {
    #[serde(default)]
    #[serde(rename = "hls-playlist-url")]
    pub hls_playlist_url: String,
    #[serde(default)]
    #[serde(rename = "hls-key-cert-url")]
    pub hls_key_cert_url: String,
    #[serde(default)]
    pub assets: Vec<WebPlaybackAsset>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WebPlaybackAsset {
    #[serde(default)]
    pub flavor: String,
    #[serde(default)]
    #[serde(rename = "URL")]
    pub url: String,
}

// ── Lyrics types ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SongLyricsResponse {
    #[serde(default)]
    pub data: Vec<SongLyricsData>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SongLyricsData {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    #[serde(rename = "type")]
    pub data_type: String,
    #[serde(default)]
    pub attributes: SongLyricsAttributes,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SongLyricsAttributes {
    #[serde(default)]
    pub ttml: String,
    #[serde(default)]
    #[serde(rename = "ttmlLocalizations")]
    pub ttml_localizations: String,
}

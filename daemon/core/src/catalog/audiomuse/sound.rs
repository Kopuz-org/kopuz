//! "Sounds like": the library albums and artists AudioMuse-AI hears as close, and what an album or artist sounds like.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::PoisonError;

use api::{ApiError, SimilarArtist, SoundView};
use futures_util::StreamExt as _;
use server::audiomuse::{AudioMuse, Score, SimilarArtist as SimilarArtistHit};

use super::audiomuse_error;
use crate::catalog::{CatalogService, db_error};

/// Songs of an album or artist that stand for it.
const SAMPLE_TRACKS: usize = 4;
/// Neighbours asked for per sampled song.
const SIMILAR_PER_TRACK: usize = 20;
/// Neighbouring artists asked for: most of the closest are one-song credits that
/// [`MIN_ARTIST_TRACKS`] drops, so ask for many to keep enough.
const SIMILAR_ARTISTS_ASKED: usize = 100;
const MAX_ALBUMS: usize = 8;
const MAX_ARTISTS: usize = 10;

/// Fewer songs than this is a guest spot or a stray tag rather than an artist to explore.
const MIN_ARTIST_TRACKS: u32 = 3;
const MAX_TAGS: usize = 3;
const MAX_MOODS: usize = 2;
/// A mood stands out when it beats the mean of all moods by this much; they cluster near 0.5 to 0.65.
const MOOD_MARGIN: f32 = 0.04;
/// Songs asked about at once.
const CONCURRENT_SAMPLES: usize = 4;
const CACHE_ENTRIES: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Album,
    Artist,
}

type Key = (Kind, config::Source, String);

/// The address and token the answers were fetched with.
type Login = (String, String);

/// Answers by what was asked, oldest out first; all of them go when the login changes.
#[derive(Default)]
pub(in crate::catalog) struct SoundCache {
    login: Login,
    by_key: HashMap<Key, SoundView>,
    order: VecDeque<Key>,
}

impl SoundCache {
    fn adopt(&mut self, login: &Login) {
        if self.login == *login {
            return;
        }
        self.by_key.clear();
        self.order.clear();
        self.login = login.clone();
    }

    fn get(&mut self, login: &Login, key: &Key) -> Option<SoundView> {
        self.adopt(login);
        self.by_key.get(key).cloned()
    }

    fn put(&mut self, login: &Login, key: Key, view: SoundView) {
        self.adopt(login);
        self.order.retain(|held| *held != key);
        self.order.push_back(key.clone());
        self.by_key.insert(key, view);
        while self.order.len() > CACHE_ENTRIES {
            if let Some(oldest) = self.order.pop_front() {
                self.by_key.remove(&oldest);
            }
        }
    }
}

/// `count` items spread evenly over `items`, all of them when there are no more than that.
fn spread<T: Clone>(items: &[T], count: usize) -> Vec<T> {
    if items.len() <= count {
        return items.to_vec();
    }
    (0..count)
        .map(|at| items[at * items.len() / count].clone())
        .collect()
}

/// Albums by the summed closeness of their songs among `hits` (album id, distance), the album asked about left out.
fn rank_albums<'a>(
    hits: impl IntoIterator<Item = (&'a str, f32)>,
    own: &str,
    limit: usize,
) -> Vec<String> {
    let mut scores: HashMap<&str, f32> = HashMap::new();
    for (album, distance) in hits {
        if album != own {
            *scores.entry(album).or_default() += 1.0 / (1.0 + distance.max(0.0));
        }
    }
    let mut ranked: Vec<(&str, f32)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    ranked
        .into_iter()
        .take(limit)
        .map(|(album, _)| album.to_string())
        .collect()
}

/// The library's artists among AudioMuse's, closest first, the artist asked about left out.
fn library_artists(
    mut found: Vec<SimilarArtistHit>,
    linked: &HashMap<String, String>,
    own: &str,
    limit: usize,
) -> Vec<SimilarArtist> {
    found.sort_by(|a, b| a.divergence.total_cmp(&b.divergence));
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter(|hit| hit.tracks.is_none_or(|tracks| tracks >= MIN_ARTIST_TRACKS))
        .filter_map(|SimilarArtistHit { id, name, .. }| {
            let key = linked.get(&id)?;
            if key == own || !seen.insert(key.clone()) {
                return None;
            }
            Some(SimilarArtist {
                name,
                key: Some(key.clone()),
            })
        })
        .take(limit)
        .collect()
}

/// Each label's mean over `scores`, a song without it counting as zero; highest first.
fn averages<'a>(
    rows: impl Iterator<Item = &'a [(String, f32)]>,
    songs: usize,
) -> Vec<(String, f32)> {
    let mut sums: HashMap<&str, f32> = HashMap::new();
    for (label, value) in rows.flatten() {
        *sums.entry(label).or_default() += value;
    }
    let mut means: Vec<(String, f32)> = sums
        .into_iter()
        .map(|(label, sum)| (label.to_string(), sum / songs as f32))
        .collect();
    means.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    means
}

/// The strongest tags, and the moods well above the mean of all of them.
fn profile(scores: &[Score]) -> (Vec<String>, Vec<String>) {
    let tags = averages(
        scores.iter().map(|score| score.tags.as_slice()),
        scores.len(),
    )
    .into_iter()
    .take(MAX_TAGS)
    .map(|(label, _)| label)
    .collect();
    let moods = averages(
        scores.iter().map(|score| score.moods.as_slice()),
        scores.len(),
    );
    let mean = moods.iter().map(|(_, value)| value).sum::<f32>() / moods.len().max(1) as f32;
    let standing = moods
        .into_iter()
        .filter(|(_, value)| value - mean >= MOOD_MARGIN)
        .take(MAX_MOODS)
        .map(|(label, _)| label)
        .collect();
    (tags, standing)
}

/// Runs `ask` for every id, a few at a time, in order; the first failure ends it.
async fn ask_each<T, Fut>(ids: &[String], ask: impl Fn(String) -> Fut) -> Result<Vec<T>, ApiError>
where
    Fut: Future<Output = Result<T, ApiError>>,
{
    futures_util::stream::iter(ids.iter().cloned())
        .map(ask)
        .buffered(CONCURRENT_SAMPLES)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect()
}

/// What AudioMuse says of one song: its neighbours and its score, `None` when it has not analysed it.
async fn sample_song(
    client: &AudioMuse,
    id: &str,
) -> Result<(Vec<(String, f32)>, Option<Score>), ApiError> {
    let (hits, score) = futures_util::future::join(
        client.similar_tracks(id, SIMILAR_PER_TRACK),
        client.score(id),
    )
    .await;
    Ok((
        hits.map_err(audiomuse_error)?,
        score.map_err(audiomuse_error)?,
    ))
}

impl CatalogService {
    fn cached_sound(&self, login: &Login, key: &Key) -> Option<SoundView> {
        let mut cache = self.sounds.lock().unwrap_or_else(PoisonError::into_inner);
        cache.get(login, key)
    }

    fn remember_sound(&self, login: &Login, key: Key, view: &SoundView) {
        let mut cache = self.sounds.lock().unwrap_or_else(PoisonError::into_inner);
        cache.put(login, key, view.clone());
    }

    /// Empty without AudioMuse-AI or when it has analysed none of the album's sampled songs.
    pub async fn album_sound(&self, album: &str) -> Result<SoundView, ApiError> {
        let Some(login) = self.audiomuse_login() else {
            return Ok(SoundView::default());
        };
        let source = self.session.sources().active.clone();
        let key = (Kind::Album, source.clone(), album.to_string());
        if let Some(view) = self.cached_sound(&login, &key) {
            return Ok(view);
        }

        let tracks = self
            .db
            .album_tracks(&source, album)
            .await
            .map_err(db_error)?;
        let keys: Vec<String> = tracks
            .iter()
            .map(|track| track.id.key().into_owned())
            .collect();
        let client = AudioMuse::new(&login.0, &login.1).map_err(audiomuse_error)?;
        let client = &client;
        let sampled = ask_each(&spread(&keys, SAMPLE_TRACKS), |id| async move {
            sample_song(client, &id).await
        })
        .await?;

        let (neighbours, scores): (Vec<_>, Vec<_>) = sampled
            .into_iter()
            .filter_map(|(hits, score)| Some((hits, score?)))
            .unzip();
        if scores.is_empty() {
            return Ok(SoundView::default());
        }

        let neighbours = neighbours.concat();
        let (tags, moods) = profile(&scores);
        let view = SoundView {
            similar_albums: self.albums_of(&source, &neighbours, album).await?,
            tags,
            moods,
            ..Default::default()
        };
        self.remember_sound(&login, key, &view);
        Ok(view)
    }

    /// The library albums the songs among `neighbours` (track key, distance) sit on, best first.
    async fn albums_of(
        &self,
        source: &config::Source,
        neighbours: &[(String, f32)],
        own: &str,
    ) -> Result<Vec<api::AlbumInfo>, ApiError> {
        let mut ids: Vec<String> = neighbours.iter().map(|(id, _)| id.clone()).collect();
        ids.sort();
        ids.dedup();
        let tracks = self
            .db
            .tracks_by_keys(source, &ids)
            .await
            .map_err(db_error)?;
        let album_of: HashMap<String, &str> = tracks
            .iter()
            .filter(|track| !track.album_id.is_empty())
            .map(|track| (track.id.key().into_owned(), track.album_id.as_str()))
            .collect();

        let hits = neighbours
            .iter()
            .filter_map(|(id, distance)| Some((*album_of.get(id)?, *distance)));
        let mut albums = Vec::new();
        for id in rank_albums(hits, own, MAX_ALBUMS) {
            let row = self.db.album(source, &id).await.map_err(db_error)?;
            albums.extend(row.as_ref().map(crate::library::album_info));
        }
        Ok(albums)
    }

    /// Empty without AudioMuse-AI, for an artist the source issued no id for, or when none of the sampled songs is analysed.
    pub async fn artist_sound(&self, artist: &str) -> Result<SoundView, ApiError> {
        let Some(login) = self.audiomuse_login() else {
            return Ok(SoundView::default());
        };
        let row = self.library.artist_row(artist).await?;
        let Some(source_id) = row.source_id.as_deref() else {
            return Ok(SoundView::default());
        };
        let source = self.session.sources().active.clone();
        let key = (Kind::Artist, source.clone(), row.key.clone());
        if let Some(view) = self.cached_sound(&login, &key) {
            return Ok(view);
        }

        let tracks = self
            .db
            .artist_tracks(&source, &row.key, None)
            .await
            .map_err(db_error)?;
        let keys: Vec<String> = tracks
            .iter()
            .map(|track| track.id.key().into_owned())
            .collect();
        let client = AudioMuse::new(&login.0, &login.1).map_err(audiomuse_error)?;
        let client = &client;
        let (similar, scored) = futures_util::future::join(
            client.similar_artists(source_id, SIMILAR_ARTISTS_ASKED),
            ask_each(&spread(&keys, SAMPLE_TRACKS), |id| async move {
                client.score(&id).await.map_err(audiomuse_error)
            }),
        )
        .await;
        let similar = similar.map_err(audiomuse_error)?;
        let scores: Vec<Score> = scored?.into_iter().flatten().collect();
        if scores.is_empty() {
            return Ok(SoundView::default());
        }

        let linked = self
            .db
            .linked_artist_keys(&source)
            .await
            .map_err(db_error)?;
        let (tags, moods) = profile(&scores);
        let view = SoundView {
            similar_artists: library_artists(similar, &linked, &row.key, MAX_ARTISTS),
            tags,
            moods,
            ..Default::default()
        };
        self.remember_sound(&login, key, &view);
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(values: &[(&str, f32)]) -> Vec<(String, f32)> {
        values
            .iter()
            .map(|(label, value)| (label.to_string(), *value))
            .collect()
    }

    fn score(tags: &[(&str, f32)], moods: &[(&str, f32)]) -> Score {
        Score {
            tags: pairs(tags),
            moods: pairs(moods),
        }
    }

    const FLAT_MOODS: [(&str, f32); 6] = [
        ("danceable", 0.59),
        ("aggressive", 0.65),
        ("happy", 0.60),
        ("party", 0.61),
        ("relaxed", 0.51),
        ("sad", 0.56),
    ];

    #[test]
    fn albums_rank_by_summed_closeness_and_never_include_their_own() {
        let hits = [
            ("own", 0.0),
            ("far", 0.9),
            ("near", 0.1),
            ("many", 0.5),
            ("many", 0.5),
            ("many", 0.5),
            ("own", 0.1),
        ];

        let ranked = rank_albums(hits, "own", 2);

        // three middling hits (3 / 1.5) beat one near hit (1 / 1.1)
        assert_eq!(ranked, ["many", "near"]);
        assert_eq!(rank_albums(hits, "own", 10), ["many", "near", "far"]);
    }

    #[test]
    fn equal_albums_rank_in_a_stable_order() {
        let ranked = rank_albums([("b", 0.2), ("a", 0.2)], "x", 8);
        assert_eq!(ranked, ["a", "b"]);
    }

    fn hit(id: &str, name: &str, divergence: f32, tracks: Option<u32>) -> SimilarArtistHit {
        SimilarArtistHit {
            id: id.into(),
            name: name.into(),
            divergence,
            tracks,
        }
    }

    #[test]
    fn an_artist_with_a_song_or_two_is_left_out() {
        let linked = HashMap::from([
            ("ar-2".to_string(), "k2".to_string()),
            ("ar-3".to_string(), "k3".to_string()),
        ]);
        // A guest spot, such as "A feat. B", is an artist of its own with one song.
        let found = vec![
            hit("ar-2", "A feat. B", 0.1, Some(2)),
            hit("ar-3", "Cyd", 0.5, Some(3)),
        ];

        let kept = library_artists(found, &linked, "k1", 10);

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].name, "Cyd");
    }

    #[test]
    fn only_library_artists_are_kept_closest_first_without_the_artist_itself() {
        let linked = HashMap::from([
            ("ar-1".to_string(), "k1".to_string()),
            ("ar-2".to_string(), "k2".to_string()),
            ("ar-3".to_string(), "k3".to_string()),
        ]);
        let found = vec![
            hit("ar-3", "Cyd", 0.9, Some(3)),
            hit("ar-9", "Elsewhere", 0.1, Some(9)),
            hit("ar-1", "Ada", 0.0, Some(9)),
            hit("ar-2", "Boris", 0.4, None),
        ];

        let kept = library_artists(found.clone(), &linked, "k1", 10);

        let keys: Vec<_> = kept.iter().map(|artist| artist.key.as_deref()).collect();
        assert_eq!(keys, [Some("k2"), Some("k3")]);
        assert_eq!(kept[0].name, "Boris");
        assert_eq!(library_artists(found, &linked, "k1", 1).len(), 1);
    }

    #[test]
    fn songs_are_sampled_evenly_across_the_album() {
        let songs: Vec<u32> = (0..10).collect();
        assert_eq!(spread(&songs, 4), [0, 2, 5, 7]);
        assert_eq!(spread(&songs[..3], 4), [0, 1, 2]);
    }

    #[test]
    fn tags_are_the_strongest_averages_and_a_missing_tag_counts_as_zero() {
        let scores = [
            score(&[("metal", 0.8), ("rock", 0.4), ("doom", 0.3)], &FLAT_MOODS),
            score(
                &[("metal", 0.6), ("sludge", 0.5), ("folk", 0.1)],
                &FLAT_MOODS,
            ),
        ];

        let (tags, _) = profile(&scores);

        // metal 0.7, rock 0.2, sludge 0.25, doom 0.15, folk 0.05
        assert_eq!(tags, ["metal", "sludge", "rock"]);
    }

    #[test]
    fn a_mood_shows_only_when_it_stands_out_from_the_mean() {
        let (_, moods) = profile(&[score(&[], &FLAT_MOODS)]);
        // mean 0.587: only aggressive (0.65) is 0.04 above it
        assert_eq!(moods, ["aggressive"]);

        let flat = [
            ("danceable", 0.55),
            ("aggressive", 0.56),
            ("happy", 0.57),
            ("party", 0.55),
            ("relaxed", 0.56),
            ("sad", 0.57),
        ];
        let (_, moods) = profile(&[score(&[], &flat)]);
        assert!(moods.is_empty());
    }

    #[test]
    fn at_most_two_moods_show_strongest_first() {
        let moods = [
            ("danceable", 0.9),
            ("aggressive", 0.8),
            ("happy", 0.7),
            ("party", 0.1),
            ("relaxed", 0.1),
            ("sad", 0.1),
        ];
        let (_, shown) = profile(&[score(&[], &moods)]);
        assert_eq!(shown, ["danceable", "aggressive"]);
    }

    #[test]
    fn moods_are_averaged_over_the_sampled_songs() {
        let calm = [
            ("danceable", 0.5),
            ("aggressive", 0.5),
            ("happy", 0.5),
            ("party", 0.5),
            ("relaxed", 0.8),
            ("sad", 0.5),
        ];
        let tense = [("aggressive", 0.9), ("relaxed", 0.2)];
        let (_, shown) = profile(&[score(&[], &calm), score(&[], &tense)]);
        // averages: aggressive 0.7, relaxed 0.5, the rest 0.25 (a missing mood counts as zero)
        assert_eq!(shown, ["aggressive", "relaxed"]);
    }

    fn key(name: &str) -> Key {
        (Kind::Album, config::Source::default(), name.to_string())
    }

    fn login(token: &str) -> Login {
        ("http://muse.example".to_string(), token.to_string())
    }

    fn view(tag: &str) -> SoundView {
        SoundView {
            tags: vec![tag.to_string()],
            ..Default::default()
        }
    }

    #[test]
    fn the_cache_forgets_its_oldest_entry_first() {
        let mut cache = SoundCache::default();
        let login = login("t");
        for at in 0..=CACHE_ENTRIES {
            cache.put(&login, key(&at.to_string()), view("x"));
        }

        assert_eq!(cache.get(&login, &key("0")), None);
        assert!(cache.get(&login, &key("1")).is_some());
        assert!(
            cache
                .get(&login, &key(&CACHE_ENTRIES.to_string()))
                .is_some()
        );
        assert_eq!(cache.by_key.len(), CACHE_ENTRIES);
    }

    #[test]
    fn putting_an_entry_again_makes_it_the_newest() {
        let mut cache = SoundCache::default();
        let login = login("t");
        for at in 0..CACHE_ENTRIES {
            cache.put(&login, key(&at.to_string()), view("x"));
        }
        cache.put(&login, key("0"), view("y"));
        cache.put(&login, key("new"), view("x"));

        assert_eq!(cache.get(&login, &key("0")), Some(view("y")));
        assert_eq!(cache.get(&login, &key("1")), None);
    }

    #[test]
    fn a_changed_login_drops_every_entry() {
        let mut cache = SoundCache::default();
        cache.put(&login("old"), key("a"), view("x"));

        assert_eq!(cache.get(&login("old"), &key("a")), Some(view("x")));
        assert_eq!(cache.get(&login("new"), &key("a")), None);
        assert_eq!(cache.get(&login("old"), &key("a")), None);
    }
}

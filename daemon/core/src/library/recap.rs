//! The listening recap over a period, with artwork for the rows the library still holds.

use std::collections::HashMap;

use api::{ApiError, ArtworkRef, ListeningRecap, RecapEntry, RecapPeriod, RecapUnit};

use super::{LibraryService, db_error};

const TOP: u32 = 5;
const DAY: i64 = 86_400;

/// The window `period` reads, ending at `now` (Unix seconds) inclusive.
fn window(period: RecapPeriod, now: i64) -> db::RecapWindow {
    let days = match period {
        RecapPeriod::Day => Some(1),
        RecapPeriod::Week => Some(7),
        RecapPeriod::Month => Some(30),
        RecapPeriod::Year => Some(365),
        RecapPeriod::AllTime => None,
    };
    db::RecapWindow {
        since: days.map(|days| now - days * DAY),
        until: now + 1,
        bucket: match period.peak_unit() {
            RecapUnit::Hour => db::RecapBucket::Hour,
            RecapUnit::Day => db::RecapBucket::Day,
            RecapUnit::Month => db::RecapBucket::Month,
        },
    }
}

impl LibraryService {
    pub async fn listening_recap(&self, period: RecapPeriod) -> Result<ListeningRecap, ApiError> {
        let source = self.query_source();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs() as i64);
        let recap = self
            .db
            .listening_recap(&source, window(period, now), TOP)
            .await
            .map_err(db_error)?;

        let keys: Vec<String> = recap
            .top_tracks
            .iter()
            .filter_map(|entry| entry.key.clone())
            .collect();
        let track_art: HashMap<String, ArtworkRef> = match keys.is_empty() {
            true => HashMap::new(),
            false => self
                .db
                .tracks_by_keys(&source, &keys)
                .await
                .map_err(db_error)?
                .iter()
                .filter_map(|track| {
                    let artwork = crate::artwork::track_ref(track)?;
                    Some((track.id.key().into_owned(), artwork))
                })
                .collect(),
        };
        let top_tracks = recap
            .top_tracks
            .into_iter()
            .map(|entry| {
                let artwork = entry
                    .key
                    .as_ref()
                    .and_then(|key| track_art.get(key))
                    .cloned();
                wire_entry(entry, artwork)
            })
            .collect();

        let mut top_albums = Vec::with_capacity(recap.top_albums.len());
        for entry in recap.top_albums {
            let artwork = match &entry.key {
                Some(id) => self
                    .db
                    .album(&source, id)
                    .await
                    .map_err(db_error)?
                    .as_ref()
                    .and_then(crate::artwork::album_ref),
                None => None,
            };
            top_albums.push(wire_entry(entry, artwork));
        }

        let mut top_artists = Vec::with_capacity(recap.top_artists.len());
        for entry in recap.top_artists {
            let artwork = match &entry.key {
                Some(key) => self.artist_artwork(key).await?,
                None => None,
            };
            top_artists.push(wire_entry(entry, artwork));
        }

        Ok(ListeningRecap {
            period,
            listened_ms: recap.listened_ms,
            plays: recap.plays,
            tracks: recap.tracks,
            artists: recap.artists,
            albums: recap.albums,
            top_tracks,
            top_artists,
            top_albums,
            peak: recap.peak.map(|peak| api::RecapPeak {
                unit: period.peak_unit(),
                year: u16::try_from(peak.year).unwrap_or_default(),
                month: peak.month,
                day: peak.day,
                hour: peak.hour,
                plays: peak.plays,
            }),
            longest_streak_days: recap.longest_streak_days,
        })
    }
}

fn wire_entry(entry: db::RecapEntry, artwork: Option<ArtworkRef>) -> RecapEntry {
    RecapEntry {
        key: entry.key,
        title: entry.title,
        subtitle: entry.subtitle,
        plays: entry.plays,
        listened_ms: entry.listened_ms,
        artwork,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_period_reads_back_from_now() {
        let now = 1_773_144_000;
        let since = |period| window(period, now).since;
        assert_eq!(since(RecapPeriod::Day), Some(now - DAY));
        assert_eq!(since(RecapPeriod::Week), Some(now - 7 * DAY));
        assert_eq!(since(RecapPeriod::Month), Some(now - 30 * DAY));
        assert_eq!(since(RecapPeriod::Year), Some(now - 365 * DAY));
        assert_eq!(since(RecapPeriod::AllTime), None);
        assert_eq!(window(RecapPeriod::Day, now).until, now + 1);
        assert_eq!(window(RecapPeriod::Day, now).bucket, db::RecapBucket::Hour);
        assert_eq!(window(RecapPeriod::Week, now).bucket, db::RecapBucket::Day);
        assert_eq!(
            window(RecapPeriod::AllTime, now).bucket,
            db::RecapBucket::Month
        );
    }
}

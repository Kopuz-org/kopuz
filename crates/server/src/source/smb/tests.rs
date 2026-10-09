use super::*;
use lofty::file::{FileType, TaggedFile};
use lofty::prelude::Accessor;
use lofty::properties::FileProperties;
use lofty::tag::{ItemKey, Tag, TagType};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn smb_metadata_preserves_tags_and_groups_album_artists() {
    let mut tag = Tag::new(TagType::VorbisComments);
    tag.set_title("First Track".into());
    tag.set_artist("Guest Artist".into());
    tag.set_album("Compilation".into());
    tag.set_track(3);
    tag.set_disk(2);
    tag.insert_text(ItemKey::AlbumArtist, "Various Artists".into());
    tag.insert_text(ItemKey::RecordingDate, "2011-01-24".into());
    let properties = FileProperties::new(
        std::time::Duration::from_millis(180_500),
        None,
        None,
        Some(44_100),
        None,
        None,
        None,
    );
    let file = TaggedFile::new(FileType::Flac, properties, vec![tag]);
    let first = metadata("Compilation/03.flac", &file, 4_320_000);
    assert_eq!(first.track.title, "First Track");
    assert_eq!(first.track.artist, "Guest Artist");
    assert_eq!(first.track.track_number, Some(3));
    assert_eq!(first.track.disc_number, Some(2));
    assert_eq!(first.album.artist, "Various Artists");
    assert_eq!(first.album.year, 2011);
    assert_eq!(first.album.genre, "Unknown");
    assert_eq!(first.track.duration, 181);
    assert_eq!(first.track.khz, 44_100);
    assert_eq!(first.track.bitrate, 192);
    assert_eq!(first.track.id.uid(), "smb:Compilation/03.flac");
    assert_eq!(
        first.track.cover.as_deref(),
        Some(reader::CoverRef::NO_COVER)
    );
    let mut second_file = file;
    second_file
        .primary_tag_mut()
        .unwrap()
        .set_artist("Another Artist".into());
    let second = metadata("Compilation/04.flac", &second_file, 0);
    assert_eq!(first.album.id, second.album.id);
}

#[test]
fn smb_untagged_tracks_use_filename_and_parent_album() {
    let file = TaggedFile::new(FileType::Flac, FileProperties::default(), Vec::new());
    let first = metadata("Artist/Album/01.flac", &file, 0);
    let second = metadata("Artist/Album/02.flac", &file, 0);
    let other = metadata("Artist/Other Album/01.flac", &file, 0);
    assert_eq!(first.track.title, "01");
    assert_eq!(first.track.artist, "Unknown Artist");
    assert_eq!(first.album.year, 0);
    assert_eq!(first.album.id, second.album.id);
    assert_ne!(first.album.id, other.album.id);
    assert_eq!(
        reader::CoverRef::for_track(&first.track),
        reader::CoverRef::None
    );
}

#[test]
fn smb_rescans_only_new_or_incomplete_tracks() {
    let file = TaggedFile::new(FileType::Flac, FileProperties::default(), Vec::new());
    let mut known = metadata("Album/known.flac", &file, 0);
    known.track.duration = 60;
    let incomplete = metadata("Album/incomplete.flac", &file, 0);
    let mut legacy = metadata("Album/legacy.flac", &file, 0);
    legacy.track.duration = 60;
    legacy.track.album_id = "smb:alb_legacy".into();
    let library = reader::Library {
        albums: vec![known.album],
        tracks: vec![known.track, incomplete.track, legacy.track],
        ..Default::default()
    };
    let paths = [
        "Album/known.flac",
        "Album/incomplete.flac",
        "Album/legacy.flac",
        "Album/new.flac",
    ]
    .map(str::to_owned)
    .to_vec();
    assert_eq!(
        scan_candidates(paths, &library),
        [
            "Album/incomplete.flac",
            "Album/legacy.flac",
            "Album/new.flac"
        ]
    );
}

#[test]
fn smb_cover_indexing_uses_one_track_per_album_and_preserves_manual_covers() {
    let file = TaggedFile::new(FileType::Flac, FileProperties::default(), Vec::new());
    let first = metadata("Album/01.flac", &file, 0);
    let second = metadata("Album/02.flac", &file, 0);
    let mut manual = metadata("Manual/01.flac", &file, 0);
    manual.album.manual_cover = true;
    let tracks = vec![first.track, second.track, manual.track];
    let candidates = cover_candidates(&[first.album, manual.album], &tracks);
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].1, "Album/01.flac");
}

struct MeasuredStream {
    data: Cursor<Vec<u8>>,
    read: Arc<AtomicUsize>,
    error: Option<SourceError>,
}

impl Read for MeasuredStream {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if let Some(error) = &self.error {
            return Err(std::io::Error::other(error.clone()));
        }
        let count = self.data.read(output)?;
        self.read.fetch_add(count, Ordering::Relaxed);
        Ok(count)
    }
}

impl Seek for MeasuredStream {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        self.data.seek(from)
    }
}

impl symphonia::core::io::MediaSource for MeasuredStream {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.data.get_ref().len() as u64)
    }
}

#[test]
fn smb_indexes_flac_without_downloading_large_embedded_art() {
    let mut bytes = b"fLaC\x00\x00\x00\x22".to_vec();
    bytes.extend_from_slice(&[0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0]);
    let properties = (44_100u64 << 44) | (1 << 41) | (15 << 36) | 44_100;
    bytes.extend_from_slice(&properties.to_be_bytes());
    bytes.extend_from_slice(&[0; 16]);
    bytes.extend_from_slice(&[0x86, 0x90, 0, 0]);
    bytes.resize(bytes.len() + 9 * 1024 * 1024, 0);
    let file = tempfile::Builder::new().suffix(".flac").tempfile().unwrap();
    std::fs::write(file.path(), &bytes).unwrap();
    let local = reader::read_metadata(file.path()).unwrap();
    let read = Arc::new(AtomicUsize::new(0));
    let remote = scan_stream(
        MeasuredStream {
            data: Cursor::new(bytes),
            read: read.clone(),
            error: None,
        },
        "Album/song.flac",
    )
    .unwrap()
    .unwrap();
    assert_eq!(remote.track.duration, 1);
    assert_eq!(remote.track.duration, local.track.duration);
    assert_eq!(remote.track.khz, local.track.khz);
    assert_eq!(remote.track.bitrate, local.track.bitrate);
    assert!(remote.album.cover_path.is_none());
    assert!(read.load(Ordering::Relaxed) < 1024);
}

#[test]
fn smb_connection_errors_abort_instead_of_dropping_tracks() {
    for path in ["song.flac", "song.mka"] {
        let result = scan_stream(
            MeasuredStream {
                data: Cursor::new(vec![0; 8192]),
                read: Arc::new(AtomicUsize::new(0)),
                error: Some(SourceError::Connectivity),
            },
            path,
        );
        assert!(matches!(result, Err(SourceError::Connectivity)));
    }
    assert!(
        scan_stream(Cursor::new(vec![0; 4096]), "broken.flac")
            .unwrap()
            .is_none()
    );
}

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use api::{ApiError, JobKind, JobRef};

use super::DownloadsService;
use crate::jobs::{JobCtx, JobRunner};

fn io_error(error: impl std::fmt::Display) -> ApiError {
    ApiError::internal(format!("CD rip failed: {error}"))
}

impl DownloadsService {
    pub async fn spawn_rip(
        self: &Arc<Self>,
        runner: &JobRunner,
        keys: Vec<String>,
        output_dir: String,
    ) -> Result<JobRef, ApiError> {
        let output_dir = PathBuf::from(output_dir);
        if !output_dir.is_absolute() {
            return Err(ApiError::invalid_input(
                "Choose an absolute output directory on the daemon host",
            ));
        }
        if keys.is_empty() || keys.len() > 99 {
            return Err(ApiError::invalid_input("Choose between 1 and 99 CD tracks"));
        }
        let config = self.session.config_watch().borrow().clone();
        let source: server::source::ActiveSource =
            Arc::from(server::source::active(self.db.clone(), &config));
        if !source.rip_audio() {
            return Err(ApiError::unsupported("ripping from this source"));
        }
        let ffmpeg = crate::url_download::find_binary("ffmpeg")
            .ok_or_else(|| ApiError::unsupported("Install ffmpeg to rip audio CDs to FLAC"))?;
        let mut tracks = self
            .db
            .tracks_by_keys(&config.active_source, &keys)
            .await
            .map_err(io_error)?;
        tracks.sort_by_key(|track| track.track_number);
        tracks.dedup_by(|left, right| left.id == right.id);
        if tracks.len() != keys.iter().collect::<std::collections::HashSet<_>>().len() {
            return Err(ApiError::invalid_input(
                "Some CD tracks are no longer in this source; refresh the disc",
            ));
        }
        let service = self.clone();
        runner.start(JobKind::AudioRip, move |ctx| async move {
            let mut artwork = RipArtwork {
                service: service.artwork.get().cloned(),
                config,
                cached: None,
            };
            // Optical drives are shared hardware. Release the playback reader before ripping.
            service.session.reset_playback().await?;
            tokio::fs::create_dir_all(&output_dir)
                .await
                .map_err(io_error)?;
            let total = tracks.len() as u64;
            for (index, track) in tracks.into_iter().enumerate() {
                if ctx.cancelled() {
                    return Ok(());
                }
                ctx.progress(
                    "ripping",
                    Some(index as u64),
                    Some(total),
                    Some(track.title.clone()),
                );
                rip_track(&ctx, &source, track, &output_dir, &ffmpeg, &mut artwork).await?;
            }
            ctx.progress(
                "done",
                Some(total),
                Some(total),
                Some(output_dir.to_string_lossy().into_owned()),
            );
            Ok(())
        })
    }
}

struct RipArtwork {
    service: Option<Arc<crate::ArtworkService>>,
    config: config::AppConfig,
    // Remember misses too, so an unavailable cover doesn't delay every track.
    cached: Option<(reader::CoverRef, Option<Vec<u8>>)>,
}

impl RipArtwork {
    async fn stage(
        &mut self,
        track: &reader::Track,
        directory: &Path,
    ) -> Result<Option<PathBuf>, ApiError> {
        let cover = reader::CoverRef::for_track(track);
        let Some(service) = &self.service else {
            return Ok(None);
        };
        if cover == reader::CoverRef::None {
            return Ok(None);
        }
        if self.cached.as_ref().is_none_or(|(key, _)| key != &cover) {
            let bytes = match service.fetch_cover(&self.config, cover.clone(), true).await {
                Ok(payload) => tokio::task::spawn_blocking(move || {
                    // Decode before handing it to the encoder: a stale URL or bad
                    // response must not turn a successful audio read into a failed rip.
                    utils::artwork_image::shrink_jpeg(&payload.bytes, 1200, 90)
                })
                .await
                .ok()
                .flatten(),
                Err(error) => {
                    tracing::debug!(%error, "CD cover unavailable; ripping without artwork");
                    None
                }
            };
            self.cached = Some((cover, bytes));
        }
        let Some((_, Some(bytes))) = &self.cached else {
            return Ok(None);
        };
        let path = directory.join("cover.jpg");
        tokio::fs::write(&path, bytes).await.map_err(io_error)?;
        Ok(Some(path))
    }
}

async fn rip_track(
    ctx: &JobCtx,
    source: &server::source::ActiveSource,
    track: reader::Track,
    directory: &Path,
    ffmpeg: &str,
    artwork: &mut RipArtwork,
) -> Result<(), ApiError> {
    let key = track.id.key().into_owned();
    let destination = destination(directory, &key)?;
    if tokio::fs::try_exists(&destination)
        .await
        .map_err(io_error)?
    {
        return Ok(());
    }
    // Reopening validates the disc-specific key before every track, including after a swap.
    let stream = source
        .open_stream(&key, None)
        .await
        .map_err(io_error)?
        .ok_or_else(|| ApiError::unsupported("native audio extraction"))?;
    let expected = stream
        .byte_len()
        .ok_or_else(|| ApiError::invalid_input("CD track length is unknown"))?;
    let staging = Arc::new(tempfile::tempdir_in(directory).map_err(io_error)?);
    let wav = staging.path().join("track.wav");
    let worker_staging = staging.clone();
    let worker_ctx = ctx.clone();
    let title = track.title.clone();
    let worker_wav = wav.clone();
    tokio::task::spawn_blocking(move || {
        // Keep the temporary directory alive even if the calling future is dropped.
        let _staging = worker_staging;
        let mut stream = stream;
        let mut output =
            std::io::BufWriter::new(std::fs::File::create(worker_wav).map_err(io_error)?);
        copy_audio(
            &mut stream,
            &mut output,
            expected,
            || worker_ctx.cancelled(),
            |copied| {
                worker_ctx.progress_throttled(
                    "ripping",
                    Some(copied),
                    Some(expected),
                    Some(title.clone()),
                )
            },
        )
        .map_err(io_error)
    })
    .await
    .map_err(io_error)??;
    if ctx.cancelled() {
        return Ok(());
    }
    let flac = tempfile::Builder::new()
        .suffix(".flac")
        .tempfile_in(directory)
        .map_err(io_error)?;
    // Recognition may finish while the drive is extracting this track.
    let track = source
        .db()
        .tracks_by_keys(source.source(), std::slice::from_ref(&key))
        .await
        .map_err(io_error)?
        .pop()
        .unwrap_or(track);
    let album = source
        .db()
        .album(source.source(), &track.album_id)
        .await
        .map_err(io_error)?;
    let cover = artwork.stage(&track, staging.path()).await?;
    if ctx.cancelled() {
        return Ok(());
    }
    let mut command = flac_command(
        ffmpeg,
        &wav,
        flac.path(),
        &track,
        album.as_ref(),
        cover.as_deref(),
    );
    ctx.progress("encoding", None, None, Some(track.title.clone()));
    let error_log = staging.path().join("encoder.log");
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(&error_log).map_err(io_error)?);
    let mut child = command.spawn().map_err(io_error)?;
    let status = loop {
        tokio::select! {
            result = child.wait() => break result.map_err(io_error)?,
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if ctx.cancelled() {
                    child.kill().await.map_err(io_error)?;
                    return Ok(());
                }
            }
        }
    };
    if !status.success() {
        let stderr = tokio::fs::read_to_string(error_log)
            .await
            .unwrap_or_default();
        tracing::warn!(%stderr, "CD FLAC encoder failed");
        return Err(io_error("FLAC encoding failed"));
    }
    if ctx.cancelled() {
        return Ok(());
    }
    flac.persist_noclobber(destination).map_err(io_error)?;
    Ok(())
}

fn flac_command(
    ffmpeg: &str,
    wav: &Path,
    flac: &Path,
    track: &reader::Track,
    album: Option<&reader::Album>,
    cover: Option<&Path>,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(ffmpeg);
    command
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(wav);
    if let Some(cover) = cover {
        command.arg("-i").arg(cover);
    }
    command
        .args([
            "-map",
            "0:a:0",
            "-c:a",
            "flac",
            "-compression_level",
            "8",
            "-sample_fmt",
            "s16",
            "-metadata",
        ])
        .arg(format!("title={}", track.title))
        .arg("-metadata")
        .arg(format!("album={}", track.album))
        .arg("-metadata")
        .arg(format!("artist={}", track.artist))
        .arg("-metadata")
        .arg(format!("track={}", track.track_number.unwrap_or_default()))
        .arg("-metadata")
        .arg(format!("disc={}", track.disc_number.unwrap_or(1)));
    for (name, value) in [
        ("MUSICBRAINZ_ALBUMID", &track.musicbrainz_release_id),
        ("MUSICBRAINZ_TRACKID", &track.musicbrainz_recording_id),
        ("MUSICBRAINZ_RELEASETRACKID", &track.musicbrainz_track_id),
    ] {
        if let Some(value) = value {
            command.arg("-metadata").arg(format!("{name}={value}"));
        }
    }
    if let Some(album) = album {
        if !album.artist.is_empty() {
            command
                .arg("-metadata")
                .arg(format!("album_artist={}", album.artist));
        }
        if album.year != 0 {
            command.arg("-metadata").arg(format!("date={}", album.year));
        }
    }
    if cover.is_some() {
        command.args([
            "-map",
            "1:v:0",
            "-c:v",
            "copy",
            "-disposition:v:0",
            "attached_pic",
            "-metadata:s:v:0",
            "title=Album cover",
            "-metadata:s:v:0",
            "comment=Cover (front)",
        ]);
    }
    command.arg(flac).kill_on_drop(true);
    command
}

fn destination(directory: &Path, key: &str) -> Result<PathBuf, ApiError> {
    // Disc identity prevents collisions between generic Track 01 names on different CDs.
    let (disc, number) = key
        .rsplit_once('-')
        .ok_or_else(|| ApiError::invalid_input("Invalid CD track key"))?;
    if disc.len() != 64
        || !disc.bytes().all(|byte| byte.is_ascii_hexdigit())
        || number.len() != 2
        || !number.bytes().all(|byte| byte.is_ascii_digit())
        || number == "00"
    {
        return Err(ApiError::invalid_input("Invalid CD track key"));
    }
    Ok(directory.join(format!("{key}.flac")))
}

fn copy_audio(
    reader: &mut impl Read,
    writer: &mut impl Write,
    expected: u64,
    cancelled: impl Fn() -> bool,
    progress: impl Fn(u64),
) -> std::io::Result<()> {
    let mut buffer = [0; 2352 * 16];
    let mut copied = 0;
    loop {
        if cancelled() {
            return Ok(());
        }
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        copied += count as u64;
        if copied > expected {
            return Err(std::io::Error::other("CD audio exceeds the track length"));
        }
        writer.write_all(&buffer[..count])?;
        progress(copied);
    }
    if copied != expected {
        return Err(std::io::Error::other(
            "CD audio ended before the track was complete",
        ));
    }
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[tokio::test]
    async fn audio_cd_rip_artwork_reuses_cache_and_tolerates_missing_or_invalid_images() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let root = tempfile::tempdir().unwrap();
        let db = db::init(&root.path().join("library.db")).await.unwrap();
        let config = config::AppConfig::default();
        let library = Arc::new(crate::LibraryService::new(
            db.clone(),
            config.active_source.clone(),
            Arc::new(radio::registry::StationRegistry::default()),
            root.path().join("covers"),
        ));
        let player =
            player::player::Player::try_with_sink(Box::new(player::engine::NullSink::new()))
                .unwrap();
        let session = crate::SessionHandle::spawn_with_player(
            library,
            player,
            crate::PlaybackServices::default(),
        );
        let service = crate::ArtworkService::new(db, session, root.path().join("cache"));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let image_path = root.path().join("original.jpg");
        image::DynamicImage::new_rgb8(32, 32)
            .save(&image_path)
            .unwrap();
        let jpeg = std::fs::read(&image_path).unwrap();
        let http = tokio::spawn(async move {
            for (status, body) in [
                ("404 Not Found", Vec::new()),
                ("200 OK", b"not an image".to_vec()),
                ("200 OK", jpeg),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(stream.read(&mut request).await.unwrap() > 0);
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                stream.write_all(&body).await.unwrap();
            }
        });
        let mut artwork = RipArtwork {
            service: Some(service.clone()),
            config: config.clone(),
            cached: None,
        };
        let disc = server::audio_cd::Disc {
            id: "a".repeat(64),
            tracks: vec![server::audio_cd::DiscTrack {
                number: 1,
                start: 0,
                end: 75,
                audio: true,
            }],
        };
        let mut track = server::source::audio_cd_snapshot(disc).tracks.remove(0);
        for path in ["missing", "invalid"] {
            track.cover = Some(format!("http://{address}/{path}"));
            assert!(artwork.stage(&track, root.path()).await.unwrap().is_none());
            // A remembered miss must not retry against the next response.
            assert!(artwork.stage(&track, root.path()).await.unwrap().is_none());
        }
        track.cover = Some(format!("http://{address}/cover"));
        service
            .fetch_cover(&config, reader::CoverRef::for_track(&track), true)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), http)
            .await
            .unwrap()
            .unwrap();
        // The HTTP listener is gone; ripping still succeeds using the UI's disk cache.
        let cover = artwork.stage(&track, root.path()).await.unwrap().unwrap();
        assert_eq!(image::open(cover).unwrap().width(), 32);
    }

    #[test]
    fn audio_cd_rip_rejects_truncated_audio_and_honors_cancellation() {
        let mut output = Vec::new();
        assert!(copy_audio(&mut Cursor::new([1, 2]), &mut output, 3, || false, |_| {}).is_err());
        output.clear();
        copy_audio(&mut Cursor::new([1, 2]), &mut output, 2, || true, |_| {}).unwrap();
        assert!(output.is_empty());
        copy_audio(&mut Cursor::new([1, 2]), &mut output, 2, || false, |_| {}).unwrap();
        assert_eq!(output, [1, 2]);
    }

    #[test]
    fn audio_cd_rip_filenames_are_disc_scoped_and_cannot_escape_destination() {
        let root = Path::new("output");
        assert!(destination(root, "../../track").is_err());
        assert!(destination(root, &format!("{}-00", "a".repeat(64))).is_err());
        let path = destination(root, &format!("{}-01", "a".repeat(64))).unwrap();
        assert_eq!(path.parent(), Some(root));
        assert_eq!(path.extension().unwrap(), "flac");
    }

    #[test]
    fn audio_cd_rip_publication_preserves_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("track.flac");
        std::fs::write(&dest, b"original").unwrap();
        let mut staging = tempfile::NamedTempFile::new_in(root.path()).unwrap();
        staging.write_all(b"new recording").unwrap();
        assert!(staging.persist_noclobber(&dest).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"original");
    }
}

#[cfg(test)]
mod encoder_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires the optional ffmpeg executable"]
    async fn audio_cd_flac_preserves_pcm_and_track_tags() {
        let ffmpeg = crate::url_download::find_binary("ffmpeg").expect("ffmpeg installed");
        let directory = tempfile::tempdir().unwrap();
        let raw = directory.path().join("samples.pcm");
        let wav = directory.path().join("track.wav");
        let pcm: Vec<u8> = (0..(2352 * 30 / 2))
            .flat_map(|sample| (sample as i16).to_le_bytes())
            .collect();
        std::fs::write(&raw, &pcm).unwrap();
        let input = tokio::process::Command::new(&ffmpeg)
            .args([
                "-nostdin", "-v", "error", "-f", "s16le", "-ar", "44100", "-ac", "2", "-i",
            ])
            .arg(&raw)
            .arg(&wav)
            .output()
            .await
            .unwrap();
        assert!(
            input.status.success(),
            "{}",
            String::from_utf8_lossy(&input.stderr)
        );
        let mut track = reader::metadata::read_metadata(&wav).unwrap().track;
        track.title = "CD test".into();
        track.album = "Test disc".into();
        track.artist = "Test artist".into();
        track.track_number = Some(3);
        track.disc_number = Some(2);
        track.musicbrainz_release_id = Some("11111111-1111-1111-1111-111111111111".into());
        track.musicbrainz_recording_id = Some("22222222-2222-2222-2222-222222222222".into());
        track.musicbrainz_track_id = Some("33333333-3333-3333-3333-333333333333".into());
        let flac = tempfile::Builder::new()
            .suffix(".flac")
            .tempfile_in(directory.path())
            .unwrap();
        let mut album = reader::metadata::read_metadata(&wav).unwrap().album;
        album.artist = "Compilation Artist".into();
        album.year = 2001;
        let cover_path = directory.path().join("cover.jpg");
        image::DynamicImage::new_rgb8(32, 32)
            .save(&cover_path)
            .unwrap();
        for cover in [None, Some(cover_path.as_path())] {
            let result = flac_command(&ffmpeg, &wav, flac.path(), &track, Some(&album), cover)
                .output()
                .await
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let embedded = reader::metadata::read_cover(flac.path());
            if cover.is_some() {
                let (bytes, mime) = embedded.expect("embedded front cover");
                assert_eq!(mime, "image/jpeg");
                assert_eq!(bytes, std::fs::read(&cover_path).unwrap());
            } else {
                assert!(embedded.is_none());
            }
            let scanned = reader::metadata::read_metadata(flac.path()).unwrap();
            assert_eq!(scanned.album.artist, album.artist);
            assert_eq!(scanned.album.year, album.year);
            let metadata = scanned.track;
            assert_eq!(metadata.title, track.title);
            assert_eq!(metadata.album, track.album);
            assert_eq!(metadata.artist, track.artist);
            assert_eq!(metadata.track_number, Some(3));
            assert_eq!(metadata.disc_number, Some(2));
            assert_eq!(
                metadata.musicbrainz_release_id,
                track.musicbrainz_release_id
            );
            assert_eq!(
                metadata.musicbrainz_recording_id,
                track.musicbrainz_recording_id
            );
            assert_eq!(metadata.musicbrainz_track_id, track.musicbrainz_track_id);
            let decoded = tokio::process::Command::new(&ffmpeg)
                .args(["-nostdin", "-v", "error", "-i"])
                .arg(flac.path())
                .args(["-f", "s16le", "-c:a", "pcm_s16le", "pipe:1"])
                .output()
                .await
                .unwrap();
            assert!(decoded.status.success());
            assert_eq!(decoded.stdout, pcm);
        }
    }
}

use super::*;
use symphonia::core::io::MediaSource;

struct Sectors {
    fail_at: Option<i32>,
    reads: Vec<i32>,
}
impl SectorReader for Sectors {
    fn sector(&mut self, lsn: i32, output: &mut [u8; SECTOR_BYTES]) -> io::Result<()> {
        self.reads.push(lsn);
        if self.fail_at == Some(lsn) {
            return Err(io::Error::other("disc removed"));
        }
        for bytes in output.chunks_exact_mut(2) {
            bytes.copy_from_slice(&(lsn as i16).to_le_bytes());
        }
        Ok(())
    }
}
fn track(number: u8, start: i32, end: i32, audio: bool) -> DiscTrack {
    DiscTrack {
        number,
        start,
        end,
        audio,
    }
}
fn stream() -> WavStream<Sectors> {
    WavStream::new(
        Sectors {
            fail_at: None,
            reads: vec![],
        },
        track(1, 150, 153, true),
    )
    .unwrap()
}

#[test]
fn audio_cd_identity_rejects_other_discs_and_data_tracks() {
    let disc = Disc::new(vec![track(1, 0, 150, true), track(2, 150, 300, false)]).unwrap();
    let other = Disc::new(vec![track(1, 0, 151, true), track(2, 151, 300, false)]).unwrap();
    assert_ne!(disc.id, other.id);
    assert!(other.track(&disc.key(&disc.tracks[0])).is_err());
    assert!(disc.track(&disc.key(&disc.tracks[1])).is_err());
    assert_eq!(disc.track(&disc.key(&disc.tracks[0])).unwrap().number, 1);
    assert!(disc.track("../../track").is_err());
}

#[test]
fn audio_cd_rejects_invalid_tables_of_contents() {
    for tracks in [
        vec![],
        vec![track(0, 0, 75, true)],
        vec![track(1, -1, 75, true)],
        vec![track(1, 75, 75, true)],
        vec![track(1, 0, 150, true), track(2, 100, 200, true)],
    ] {
        assert!(Disc::new(tracks).is_err());
    }
}

#[test]
fn audio_cd_wav_contains_exact_track_samples_and_no_following_track() {
    let mut stream = stream();
    let mut wav = Vec::new();
    stream.read_to_end(&mut wav).unwrap();
    assert_eq!(wav.len(), 44 + 3 * SECTOR_BYTES);
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..16], b"WAVEfmt ");
    assert_eq!(&wav[24..28], &44100u32.to_le_bytes());
    assert_eq!(&wav[40..44], &(3 * SECTOR_BYTES as u32).to_le_bytes());
    assert_eq!(stream.reader.reads, [150, 151, 152]);
    for (index, sector) in wav[44..].chunks_exact(SECTOR_BYTES).enumerate() {
        assert!(
            sector
                .chunks_exact(2)
                .all(|sample| sample == (150i16 + index as i16).to_le_bytes())
        );
    }
    assert_eq!(stream.byte_len(), Some(wav.len() as u64));
}

#[test]
fn audio_cd_seek_handles_header_unaligned_samples_and_end_of_track() {
    let mut stream = stream();
    stream
        .seek(SeekFrom::Start(44 + SECTOR_BYTES as u64 - 1))
        .unwrap();
    let mut bytes = [0; 4];
    stream.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes, [0, 151, 0, 151]);
    stream.seek(SeekFrom::End(-2)).unwrap();
    stream.read_exact(&mut bytes[..2]).unwrap();
    assert_eq!(&bytes[..2], &152i16.to_le_bytes());
    assert_eq!(stream.read(&mut bytes).unwrap(), 0);
    stream.seek(SeekFrom::Start(0)).unwrap();
    stream.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"RIFF");
    assert!(stream.seek(SeekFrom::Current(-5)).is_err());
    assert_eq!(stream.stream_position().unwrap(), 4);
    assert_eq!(stream.read(&mut []).unwrap(), 0);
}

#[test]
fn audio_cd_read_failure_does_not_become_silent_audio_or_eof() {
    let mut stream = stream();
    stream.reader.fail_at = Some(151);
    stream
        .seek(SeekFrom::Start(44 + SECTOR_BYTES as u64))
        .unwrap();
    let before = stream.stream_position().unwrap();
    assert!(stream.read(&mut [0; 2]).is_err());
    assert_eq!(stream.stream_position().unwrap(), before);
}

#[test]
fn audio_cd_wave_is_decodable_by_the_playback_demuxer() {
    let source =
        symphonia::core::io::MediaSourceStream::new(Box::new(stream()), Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(
            &Default::default(),
            source,
            Default::default(),
            Default::default(),
        )
        .unwrap();
    let params = format.tracks()[0]
        .codec_params
        .as_ref()
        .unwrap()
        .audio()
        .unwrap();
    assert_eq!(params.sample_rate, Some(44100));
    assert_eq!(params.bits_per_sample, Some(16));
    assert_eq!(params.channels.as_ref().unwrap().count(), 2);
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &Default::default())
        .unwrap();
    let mut frames = 0;
    while let Some(packet) = format.next_packet().unwrap() {
        frames += decoder.decode(&packet).unwrap().frames();
    }
    assert_eq!(frames, 3 * 588);
}

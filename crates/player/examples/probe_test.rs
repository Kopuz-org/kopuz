// Quick diagnostic: run the same probe+decode path the player uses, on a file.
use std::path::Path;
use symphonia::core::formats::probe::Hint;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::formats::{FormatOptions};
use symphonia::core::meta::MetadataOptions;

fn main() {
    let path = std::env::args().nth(1).expect("pass a file path");
    let p = Path::new(&path);
    let file = std::fs::File::open(p).expect("open");
    let mut hint = Hint::new();
    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut format = match symphonia::default::get_probe().probe(
        &hint,
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    ) {
        Ok(f) => f,
        Err(e) => {
            println!("PROBE ERROR: {e}");
            return;
        }
    };
    let track = match format
        .tracks()
        .iter()
        .find(|t| t.codec_params.as_ref().and_then(|p| p.audio()).is_some())
    {
        Some(t) => t,
        None => {
            println!("NO AUDIO TRACK FOUND");
            return;
        }
    };
    let audio_params = track.codec_params.as_ref().and_then(|p| p.audio()).unwrap().clone();
    println!("audio params: codec={:?} sr={:?} ch={:?}", audio_params.codec, audio_params.sample_rate, audio_params.channels);
    let opts = AudioDecoderOptions::default().gapless(false);
    let mut decoder: Box<dyn AudioDecoder> = match symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &opts)
    {
        Ok(d) => d,
        Err(e) => {
            println!("MAKE DECODER ERROR: {e}");
            return;
        }
    };
    // decode ALL packets, count frames, report how/when it ends
    let mut packets = 0u64;
    let mut frames = 0u64;
    let mut decode_errs = 0u64;
    loop {
        match format.next_packet() {
            Ok(Some(pkt)) => {
                packets += 1;
                match decoder.decode(&pkt) {
                    Ok(d) => frames += d.frames() as u64,
                    Err(symphonia::core::errors::Error::DecodeError(_)) => decode_errs += 1,
                    Err(e) => { println!("FATAL DECODE ERROR after {packets} packets: {e}"); break; }
                }
            }
            Ok(None) => { println!("END (Ok(None)) after {packets} packets"); break; }
            Err(symphonia::core::errors::Error::IoError(ref e)) if e.kind()==std::io::ErrorKind::UnexpectedEof => { println!("END (EOF) after {packets} packets"); break; }
            Err(e) => { println!("NEXT PACKET ERROR after {packets} packets: {e}"); break; }
        }
    }
    let secs = if audio_params.sample_rate.unwrap_or(0) > 0 { frames as f64 / audio_params.sample_rate.unwrap() as f64 } else { 0.0 };
    println!("TOTAL: {packets} packets, {frames} frames (~{secs:.1}s audio), {decode_errs} decode errors");
}

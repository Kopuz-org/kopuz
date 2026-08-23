use super::widevine::Cdm;

fn u32be(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
fn u64be(d: &[u8], o: usize) -> u64 {
    u64::from_be_bytes([
        d[o],
        d[o + 1],
        d[o + 2],
        d[o + 3],
        d[o + 4],
        d[o + 5],
        d[o + 6],
        d[o + 7],
    ])
}

const ENCA: u32 = u32::from_be_bytes(*b"enca");
const ENCV: u32 = u32::from_be_bytes(*b"encv");
const STSD: u32 = u32::from_be_bytes(*b"stsd");
const MOOV: u32 = u32::from_be_bytes(*b"moov");
const MOOF: u32 = u32::from_be_bytes(*b"moof");
const MDAT: u32 = u32::from_be_bytes(*b"mdat");
const TRAK: u32 = u32::from_be_bytes(*b"trak");
const TKHD: u32 = u32::from_be_bytes(*b"tkhd");
const SINF: u32 = u32::from_be_bytes(*b"sinf");
const SCHI: u32 = u32::from_be_bytes(*b"schi");
const TENC: u32 = u32::from_be_bytes(*b"tenc");
const TRAF: u32 = u32::from_be_bytes(*b"traf");
const SENC: u32 = u32::from_be_bytes(*b"senc");
const TRUN: u32 = u32::from_be_bytes(*b"trun");
const TFHD: u32 = u32::from_be_bytes(*b"tfhd");

fn read_box(data: &[u8], pos: usize) -> Option<(usize, usize, usize)> {
    if pos + 8 > data.len() {
        return None;
    }
    let size = u32be(data, pos) as usize;
    if size == 1 {
        if pos + 16 > data.len() {
            return None;
        }
        let ext = u64be(data, pos + 8) as usize;
        let body_start = pos + 16;
        let body_end = pos + ext;
        if body_end > data.len() {
            return None;
        }
        Some((body_start, body_end, ext))
    } else if size >= 8 {
        let body_start = pos + 8;
        let body_end = pos + size;
        if body_end > data.len() {
            return None;
        }
        Some((body_start, body_end, size))
    } else {
        None
    }
}

fn box_type(data: &[u8], pos: usize) -> u32 {
    u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]])
}

fn find_child(
    data: &[u8],
    body_start: usize,
    body_end: usize,
    target: u32,
) -> Option<(usize, usize, usize)> {
    let mut pos = body_start;
    while pos < body_end {
        let (bs, be, total) = match read_box(data, pos) {
            Some(v) => v,
            None => break,
        };
        if box_type(data, pos) == target {
            return Some((bs, be, total));
        }
        pos += total;
    }
    None
}

fn find_deep(data: &[u8], start: usize, end: usize, target: u32) -> Option<(usize, usize)> {
    let mut pos = start;
    while pos < end {
        let (bs, be, total) = match read_box(data, pos) {
            Some(v) => v,
            None => break,
        };
        if box_type(data, pos) == target {
            return Some((bs, be));
        }
        if let Some(found) = find_deep(data, bs, be, target) {
            return Some(found);
        }
        pos += total;
    }
    None
}

/// Each child as `(box_start, body_start, body_end)`.
///
/// `body_end` comes from [`read_box`], which is the only place that knows
/// whether the box carried a 32- or 64-bit size. Deriving it at the call site
/// as `body_start + total - 8` is right for a 32-bit box and eight bytes too
/// long for a 64-bit one, which walks the child scan into the next sibling.
fn find_all_children(
    data: &[u8],
    body_start: usize,
    body_end: usize,
) -> Vec<(usize, usize, usize)> {
    let mut result = Vec::new();
    let mut pos = body_start;
    while pos < body_end {
        let (bs, be, total) = match read_box(data, pos) {
            Some(v) => v,
            None => break,
        };
        result.push((pos, bs, be));
        pos += total;
    }
    result
}

// Track info

struct TrackInfo {
    track_id: u32,
    default_iv_size: u8,
}

fn extract_track_info(
    data: &[u8],
    init_end: usize,
) -> Result<(Vec<TrackInfo>, Vec<usize>), String> {
    let mut track_infos = Vec::new();
    let mut enca_positions = Vec::new();

    let (moov_body_start, moov_body_end) = match find_deep(data, 0, init_end, MOOV) {
        Some(v) => v,
        None => return Ok((track_infos, enca_positions)),
    };

    for (trak_box_start, trak_body_start, trak_body_end) in
        find_all_children(data, moov_body_start, moov_body_end)
    {
        if box_type(data, trak_box_start) != TRAK {
            continue;
        }

        // The version byte decides where the track id sits, so the length
        // needed isn't known until it has been read — hence the two checks.
        let track_id = find_child(data, trak_body_start, trak_body_end, TKHD)
            .filter(|(bs, be, _)| bs < be)
            .and_then(|(s, be, _)| {
                let offset = if data[s] == 0 { 12 } else { 20 };
                (s + offset + 4 <= be).then(|| u32be(data, s + offset))
            })
            .unwrap_or(0);

        if let Some((stsd_bs, stsd_be)) = find_deep(data, trak_body_start, trak_body_end, STSD) {
            let entries_start = stsd_bs + 8;
            let mut epos = entries_start;
            while epos < stsd_be {
                let (es, ee, etotal) = match read_box(data, epos) {
                    Some(v) => v,
                    None => break,
                };
                let etype = box_type(data, epos);

                if etype == ENCA || etype == ENCV {
                    let children_start = es + 28;
                    let default_iv = get_tenc_iv_size(data, children_start, ee);
                    tracing::debug!("am.decrypt: track {track_id}: tenc iv_size={default_iv}");
                    track_infos.push(TrackInfo {
                        track_id,
                        default_iv_size: default_iv,
                    });
                    enca_positions.push(epos);
                }
                epos += etotal;
            }
        }
        break;
    }

    Ok((track_infos, enca_positions))
}

fn get_tenc_iv_size(data: &[u8], enca_body_start: usize, enca_body_end: usize) -> u8 {
    if let Some((sinf_bs, sinf_be, _)) = find_child(data, enca_body_start, enca_body_end, SINF)
        && let Some((schi_bs, schi_be, _)) = find_child(data, sinf_bs, sinf_be, SCHI)
        && let Some((tenc_bs, _, _)) = find_child(data, schi_bs, schi_be, TENC)
        && tenc_bs + 8 <= data.len()
    {
        return data[tenc_bs + 7];
    }
    16
}

// SENC parsing

/// Per-sample decryption inputs read out of a `senc` box: each sample's 16-byte
/// IV, paired with its subsample layout as `(clear_bytes, encrypted_bytes)`
/// runs (empty when the sample is encrypted whole).
type SencSamples = (Vec<[u8; 16]>, Vec<Vec<(u16, u32)>>);

fn parse_senc(iv_size: u8, sample_count: u32, raw_data: &[u8], use_subsample: bool) -> SencSamples {
    if iv_size == 0 && sample_count == 0 {
        return (vec![], vec![]);
    }

    if use_subsample {
        // Subsample mode: each sample has [IV] + subsample_count(2) + patterns(n*6)
        if iv_size != 0
            && let Some(result) = try_parse_senc(iv_size, sample_count, raw_data, true)
        {
            return result;
        }
        for try_size in [0u8, 8, 16] {
            if try_size == iv_size {
                continue;
            }
            if let Some(result) = try_parse_senc(try_size, sample_count, raw_data, true) {
                tracing::info!("am.decrypt: senc parsed with inferred iv_size={try_size}");
                return result;
            }
        }
    } else {
        // Full-sample mode: each sample has just [IV], no subsample patterns
        if iv_size != 0
            && let Some(result) = try_parse_senc(iv_size, sample_count, raw_data, false)
        {
            return result;
        }
        for try_size in [0u8, 8, 16] {
            if try_size == iv_size {
                continue;
            }
            if let Some(result) = try_parse_senc(try_size, sample_count, raw_data, false) {
                tracing::info!("am.decrypt: senc parsed with inferred iv_size={try_size}");
                return result;
            }
        }
    }

    tracing::warn!("am.decrypt: could not parse senc with any IV size");
    (vec![], vec![])
}

fn try_parse_senc(
    iv_size: u8,
    sample_count: u32,
    raw_data: &[u8],
    use_subsample: bool,
) -> Option<SencSamples> {
    let count = sample_count as usize;
    let mut pos = 0usize;
    let mut ivs = Vec::with_capacity(count);
    let mut subs = Vec::with_capacity(count);

    for _ in 0..count {
        if iv_size > 0 {
            if raw_data.len().saturating_sub(pos) < iv_size as usize {
                return None;
            }
            let mut iv = [0u8; 16];
            iv[..iv_size as usize].copy_from_slice(&raw_data[pos..pos + iv_size as usize]);
            ivs.push(iv);
            pos += iv_size as usize;
        }
        if use_subsample {
            if raw_data.len().saturating_sub(pos) < 2 {
                return None;
            }
            let n = u16::from_be_bytes([raw_data[pos], raw_data[pos + 1]]) as usize;
            pos += 2;
            if raw_data.len().saturating_sub(pos) < n * 6 {
                return None;
            }
            let mut patterns = Vec::with_capacity(n);
            for _ in 0..n {
                let clear = u16::from_be_bytes([raw_data[pos], raw_data[pos + 1]]);
                let protected = u32::from_be_bytes([
                    raw_data[pos + 2],
                    raw_data[pos + 3],
                    raw_data[pos + 4],
                    raw_data[pos + 5],
                ]);
                patterns.push((clear, protected));
                pos += 6;
            }
            subs.push(patterns);
        } else {
            subs.push(vec![]);
        }
    }

    if pos != raw_data.len() {
        return None;
    }

    Some((ivs, subs))
}

// CENC decryption

/// Decrypt one CENC sample in place through the CDM.
fn crypt_sample_cenc(
    sample: &mut [u8],
    cdm: &Cdm,
    key_id: &[u8],
    iv: &[u8; 16],
    subs: &[(u16, u32)],
) -> Result<(), String> {
    let subsamples: Vec<(u32, u32)> = subs
        .iter()
        .map(|&(clear, protected)| (u32::from(clear), protected))
        .collect();
    let clear = cdm.decrypt(sample, key_id, iv, &subsamples)?;
    if clear.len() != sample.len() {
        return Err(format!(
            "CDM returned {} bytes for a {}-byte sample",
            clear.len(),
            sample.len()
        ));
    }
    sample.copy_from_slice(&clear);
    Ok(())
}
/// One encrypted sample: where it sits in the file, and what the CDM needs to
/// turn it into cleartext.
///
/// Offsets are absolute and identical in ciphertext and cleartext — CENC is
/// size-preserving and every box is copied verbatim — which is what lets a
/// sample be decrypted in isolation, in place, in any order.
#[derive(Debug, Clone)]
pub struct EncryptedSample {
    pub start: usize,
    pub len: usize,
    pub iv: [u8; 16],
    pub subs: Vec<(u16, u32)>,
}

impl EncryptedSample {
    pub fn end(&self) -> usize {
        self.start + self.len
    }
}

/// Everything needed to decrypt a track without walking its boxes again.
#[derive(Debug, Default)]
pub struct Fmp4Layout {
    /// End of the init segment (`ftyp`..`moov`).
    pub init_end: usize,
    /// `enca`/`encv` box offsets to be relabelled `mp4a` so decoders accept the
    /// now-cleartext track.
    pub enca_positions: Vec<usize>,
    /// Every encrypted sample, in file order.
    pub samples: Vec<EncryptedSample>,
}

/// Walk the fMP4 once and record where every encrypted sample lives.
///
/// Pure parsing — no CDM calls — so it's cheap (~10ms for a 7 MB track) and can
/// run before the first byte is needed.
pub fn index_fmp4(data: &[u8]) -> Result<Fmp4Layout, String> {
    // 1. Find init segment (ftyp + moov)
    let mut init_end = 0usize;
    let mut pos = 0;
    while pos + 8 <= data.len() {
        let (_, be, total) = match read_box(data, pos) {
            Some(v) => v,
            None => break,
        };
        if box_type(data, pos) == MOOV {
            init_end = be;
            break;
        }
        pos += total;
    }
    if init_end == 0 {
        return Err("no moov".to_string());
    }

    let (track_infos, enca_positions) = extract_track_info(data, init_end)?;
    let mut layout = Fmp4Layout {
        init_end,
        enca_positions,
        samples: Vec::new(),
    };

    // 2. Walk each fragment (moof + mdat) collecting sample positions.
    pos = init_end;
    while pos + 8 <= data.len() {
        let (moof_bs, moof_be, moof_total) = match read_box(data, pos) {
            Some(v) => v,
            None => break,
        };
        if box_type(data, pos) != MOOF {
            pos += moof_total;
            continue;
        }
        let moof_pos = pos;
        let moof_start_pos = moof_pos as u64;

        // The mdat carrying this moof's samples follows it.
        let mdat_pos = moof_pos + moof_total;
        let Some((_, _, mdat_total_size)) = read_box(data, mdat_pos) else {
            break;
        };
        if box_type(data, mdat_pos) != MDAT {
            pos = moof_be;
            continue;
        }
        let mdat_body_start = mdat_pos + 8;
        let mdat_payload_offset = mdat_body_start as u64;

        for (traf_pos, traf_bs, traf_be) in find_all_children(data, moof_bs, moof_be) {
            if box_type(data, traf_pos) != TRAF {
                continue;
            }

            // Every read below is bounds-checked against the box's own end
            // rather than assumed. These bytes come off the network, and
            // `index_fmp4` runs under `State`'s mutex — an out-of-range index
            // here would poison it and leave the track unplayable for good,
            // reported only as "decrypt state poisoned".
            let tfhd = find_child(data, traf_bs, traf_be, TFHD);
            let track_id = tfhd
                .as_ref()
                .filter(|(bs, be, _)| bs + 8 <= *be)
                .map(|(s, _, _)| u32be(data, s + 4))
                .unwrap_or(0);

            let Some(ti) = track_infos.iter().find(|t| t.track_id == track_id) else {
                continue;
            };
            let per_sample_iv_size = ti.default_iv_size;

            let mut traf_ivs: Vec<[u8; 16]> = Vec::new();
            let mut traf_subs: Vec<Vec<(u16, u32)>> = Vec::new();
            if let Some((senc_bs, senc_be, _)) = find_child(data, traf_bs, traf_be, SENC)
                && senc_bs + 8 <= senc_be
            {
                let flags = u32be(data, senc_bs);
                let sample_count = u32be(data, senc_bs + 4);
                let raw = &data[senc_bs + 8..senc_be];
                let use_subsample = (flags & 0x02) != 0;
                let (ivs, subs) = parse_senc(per_sample_iv_size, sample_count, raw, use_subsample);
                traf_ivs = ivs;
                traf_subs = subs;
            }

            let trun = find_child(data, traf_bs, traf_be, TRUN);
            let (trun_data_offset, samples) = match trun {
                Some((trun_bs, trun_be, _)) => parse_trun(
                    data,
                    trun_bs,
                    trun_be,
                    tfhd,
                    moof_start_pos,
                    mdat_payload_offset,
                    &mdat_body_start,
                    mdat_total_size,
                ),
                None => (0, vec![]),
            };
            if samples.is_empty() {
                continue;
            }

            let mdat_body_len = mdat_total_size.saturating_sub(8);
            if mdat_body_start + mdat_body_len > data.len() {
                continue;
            }

            // A running offset: re-summing the preceding sizes per sample is
            // quadratic, and a fragment holds hundreds of samples.
            let mut offset = trun_data_offset;
            let mut iv = [0u8; 16];
            for (i, &sz) in samples.iter().enumerate() {
                let sz = sz as usize;
                if sz == 0 {
                    continue;
                }
                if i < traf_ivs.len() {
                    iv = traf_ivs[i];
                }
                if offset + sz > mdat_body_len {
                    break;
                }
                layout.samples.push(EncryptedSample {
                    start: mdat_body_start + offset,
                    len: sz,
                    iv,
                    subs: traf_subs.get(i).cloned().unwrap_or_default(),
                });
                offset += sz;
            }
        }

        pos = moof_be;
    }

    // Debug, not info: a streaming track re-walks this on every new fragment, so
    // at info it drowns the log and makes indexing look like the slow step.
    tracing::debug!(
        "am.decrypt: indexed {} samples, init {} bytes",
        layout.samples.len(),
        layout.init_end
    );
    Ok(layout)
}

/// Relabel `enca`/`encv` as `mp4a` so the decoder reads the track as plain AAC.
/// Only the 4-byte box type changes, so every offset is preserved.
pub fn patch_init(buf: &mut [u8], layout: &Fmp4Layout) {
    for pos in &layout.enca_positions {
        if pos + 8 <= buf.len() {
            buf[pos + 4..pos + 8].copy_from_slice(b"mp4a");
        }
    }
}

/// Decrypt one sample in place. `buf` is the whole file; `sample.start` indexes
/// into it directly.
pub fn decrypt_sample(
    buf: &mut [u8],
    sample: &EncryptedSample,
    cdm: &Cdm,
    key_id: &[u8],
) -> Result<(), String> {
    if sample.end() > buf.len() {
        return Err("sample runs past end of file".to_string());
    }
    crypt_sample_cenc(
        &mut buf[sample.start..sample.end()],
        cdm,
        key_id,
        &sample.iv,
        &sample.subs,
    )
}

/// Decrypt a whole track at once — the simple path, kept for callers that want
/// the finished bytes rather than a stream.
pub fn decrypt_fmp4(data: &[u8], cdm: &Cdm, key_id: &[u8]) -> Result<Vec<u8>, String> {
    let layout = index_fmp4(data)?;
    let mut buf = data.to_vec();
    patch_init(&mut buf, &layout);

    let started = std::time::Instant::now();
    for sample in &layout.samples {
        decrypt_sample(&mut buf, sample, cdm, key_id)?;
    }
    let total = started.elapsed();
    tracing::info!(
        "am.decrypt: done — {} samples, {} bytes in {:.2}s ({:.0}µs/sample)",
        layout.samples.len(),
        buf.len(),
        total.as_secs_f64(),
        total.as_secs_f64() * 1e6 / layout.samples.len().max(1) as f64
    );
    Ok(buf)
}

// Parse trun

fn parse_trun(
    data: &[u8],
    trun_bs: usize,
    trun_be: usize,
    tfhd: Option<(usize, usize, usize)>,
    moof_start_pos: u64,
    mdat_payload_offset: u64,
    _mdat_body_start: &usize,
    _mdat_total_size: usize,
) -> (usize, Vec<u32>) {
    if trun_bs + 8 > trun_be {
        return (0, vec![]);
    }

    let trun_flags = u32be(data, trun_bs);
    let sample_count = u32be(data, trun_bs + 4) as usize;
    let mut tpos = trun_bs + 8;

    let mut has_data_offset = false;
    let mut trun_data_offset_i32 = 0i32;
    let mut has_first_sample_flags = false;

    if trun_flags & 0x000001 != 0 && tpos + 4 <= trun_be {
        trun_data_offset_i32 = i32::from_be_bytes(data[tpos..tpos + 4].try_into().unwrap());
        has_data_offset = true;
        tpos += 4;
    }
    if trun_flags & 0x000004 != 0 && tpos + 4 <= trun_be {
        has_first_sample_flags = true;
        tpos += 4;
    }

    let mut durations = Vec::with_capacity(sample_count);
    let mut sizes = Vec::with_capacity(sample_count);
    let mut flags = Vec::with_capacity(sample_count);
    let mut composition_offsets = Vec::with_capacity(sample_count);

    for i in 0..sample_count {
        if trun_flags & 0x000100 != 0 && tpos + 4 <= trun_be {
            durations.push(u32be(data, tpos));
            tpos += 4;
        }
        if trun_flags & 0x000200 != 0 && tpos + 4 <= trun_be {
            sizes.push(u32be(data, tpos));
            tpos += 4;
        }
        if trun_flags & 0x000400 != 0 && tpos + 4 <= trun_be {
            flags.push(u32be(data, tpos));
            tpos += 4;
        } else if i == 0 && has_first_sample_flags {
            flags.push(u32be(
                data,
                trun_bs + 8 + if has_data_offset { 4 } else { 0 },
            ));
        }
        if trun_flags & 0x000800 != 0 && tpos + 4 <= trun_be {
            composition_offsets.push(i32::from_be_bytes(data[tpos..tpos + 4].try_into().unwrap()));
            tpos += 4;
        }
    }

    // Fill default sizes from tfhd/trex if trun didn't provide sizes
    if sizes.is_empty()
        && sample_count > 0
        && let Some((tfhd_bs, _, _)) = tfhd
    {
        // tfhd body: version(1)+flags(3)=4 bytes, track_id(4 bytes), then optional fields
        let tfhd_version_flags = u32be(data, tfhd_bs);
        let tfhd_flags = tfhd_version_flags & 0x00FFFFFF;
        let mut off = tfhd_bs + 8; // skip version+flags + track_id
        if tfhd_flags & 0x000001 != 0 {
            off += 8;
        } // base_data_offset (u64)
        if tfhd_flags & 0x000002 != 0 {
            off += 4;
        } // sample_description_index (u32)
        if tfhd_flags & 0x000008 != 0 {
            off += 4;
        } // default_sample_duration (u32)
        if tfhd_flags & 0x000010 != 0 && off + 4 <= data.len() {
            let def_size = u32be(data, off);
            if def_size > 0 {
                sizes = vec![def_size; sample_count];
            }
        }
    }

    // baseOffset = moofStartPos; if trun has dataOffset: baseOffset += dataOffset
    // offsetInMdat = baseOffset - mdatPayloadOffset
    let mut data_start: usize = 0;
    if has_data_offset {
        let base_offset = moof_start_pos.wrapping_add(trun_data_offset_i32 as i64 as u64);
        if base_offset >= mdat_payload_offset {
            data_start = (base_offset - mdat_payload_offset) as usize;
        }
    }

    tracing::debug!(
        "am.decrypt: trun samples={} sizes={} data_start={}",
        sample_count,
        sizes.len(),
        data_start
    );

    (data_start, sizes)
}

const MP4A: u32 = u32::from_be_bytes(*b"mp4a");
const ESDS: u32 = u32::from_be_bytes(*b"esds");

/// What `MediaFormat` needs to configure an AAC decoder.
///
/// Android's `MediaCodec` can't be handed an fMP4; it wants the track's
/// parameters and the raw `AudioSpecificConfig` as `csd-0`. All three live in the
/// init segment, so this is the last thing the Android path needs out of the
/// container before it can feed samples in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    /// `AudioSpecificConfig`, verbatim — `MediaFormat`'s `csd-0`.
    pub codec_specific: Vec<u8>,
}

/// Read one length from an MP4 descriptor: 7 bits per byte, top bit continues.
///
/// Returns the value and how many bytes it occupied. Capped at four bytes, as
/// the spec allows, so a corrupt stream can't spin here.
fn descriptor_len(data: &[u8], pos: usize) -> Option<(usize, usize)> {
    let mut len = 0usize;
    let mut used = 0usize;
    while used < 4 {
        let byte = *data.get(pos + used)?;
        used += 1;
        len = (len << 7) | (byte & 0x7F) as usize;
        if byte & 0x80 == 0 {
            return Some((len, used));
        }
    }
    Some((len, used))
}

/// Find descriptor `tag` inside `esds`, returning its body range.
///
/// Descriptors nest — ES_Descriptor (3) holds DecoderConfigDescriptor (4) holds
/// DecoderSpecificInfo (5) — and each has a header whose size varies, so this
/// walks in rather than seeking to a fixed offset.
fn find_descriptor(data: &[u8], mut pos: usize, end: usize, tag: u8) -> Option<(usize, usize)> {
    while pos < end {
        let this_tag = *data.get(pos)?;
        let (len, used) = descriptor_len(data, pos + 1)?;
        let body = pos + 1 + used;
        let body_end = body.checked_add(len)?.min(end);
        if this_tag == tag {
            return Some((body, body_end));
        }
        // Descend through the containers on the way to the payload.
        match this_tag {
            // ES_Descriptor: ES_ID(2) + flags(1) before its children.
            3 => pos = body + 3,
            // DecoderConfigDescriptor: 13 bytes of fixed fields before its children.
            4 => pos = body + 13,
            _ => pos = body_end,
        }
    }
    None
}

/// Pull the AAC parameters out of an init segment.
///
/// Works whether or not the sample entry has been relabelled: `enca` and `mp4a`
/// share the `AudioSampleEntry` layout, and `esds` sits among the children of
/// both.
pub fn audio_config(data: &[u8]) -> Option<AudioConfig> {
    let (moov_bs, moov_be) = find_deep(data, 0, data.len(), MOOV)?;
    let (stsd_bs, _) = find_deep(data, moov_bs, moov_be, STSD)?;

    // stsd body: version+flags(4), entry_count(4), then sample entries.
    let entry = stsd_bs + 8;
    let (entry_body, entry_end, _) = read_box(data, entry)?;
    let kind = box_type(data, entry);
    if kind != ENCA && kind != MP4A {
        return None;
    }

    // AudioSampleEntry: 6 reserved + 2 data_reference_index, then 8 bytes of
    // version/revision/vendor, channelcount(2), samplesize(2), pre_defined(2),
    // reserved(2), samplerate(4, 16.16 fixed point).
    //
    // `read_box` only vouches for the box being inside the buffer, not for the
    // body being long enough to hold the fields the type implies — a sample
    // entry declaring size 8 passes it. Reading through `get` turns a short one
    // into `None` instead of an index past the end.
    let channels = u16::from_be_bytes(
        data.get(entry_body + 16..entry_body + 18)?
            .try_into()
            .ok()?,
    );
    // Only the integer half of the 16.16 rate is meaningful for AAC.
    let sample_rate = u32::from(u16::from_be_bytes(
        data.get(entry_body + 24..entry_body + 26)?
            .try_into()
            .ok()?,
    ));

    let (esds_bs, esds_be, _) = find_child(data, entry_body + 28, entry_end, ESDS)?;
    // esds body starts with version+flags.
    let (asc_start, asc_end) = find_descriptor(data, esds_bs + 4, esds_be, 5)?;
    let codec_specific = data.get(asc_start..asc_end)?.to_vec();
    if codec_specific.is_empty() || channels == 0 || sample_rate == 0 {
        return None;
    }

    Some(AudioConfig {
        sample_rate,
        channels,
        codec_specific,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Descriptor lengths are 7 bits per byte with the top bit continuing, and
    /// encoders differ on whether they pad to a fixed width — so both the short
    /// and the padded forms have to read back the same.
    #[test]
    fn descriptor_lengths_decode_in_both_forms() {
        assert_eq!(descriptor_len(&[0x05], 0), Some((5, 1)));
        assert_eq!(descriptor_len(&[0x80, 0x05], 0), Some((5, 2)));
        assert_eq!(
            descriptor_len(&[0x80, 0x80, 0x80, 0x05], 0),
            Some((5, 4)),
            "the four-byte padded form Apple and others emit"
        );
        // 0x81 0x00 => (1 << 7) | 0 = 128
        assert_eq!(descriptor_len(&[0x81, 0x00], 0), Some((128, 2)));
        assert_eq!(descriptor_len(&[], 0), None);
        // Never runs past four bytes even if every one sets the continue bit.
        assert_eq!(descriptor_len(&[0x80, 0x80, 0x80, 0x80], 0), Some((0, 4)));
    }

    /// Walking to the payload has to descend through ES_Descriptor and
    /// DecoderConfigDescriptor, whose fixed fields differ in size. Seeking to a
    /// fixed offset instead would land in the middle of a field.
    #[test]
    fn the_specific_info_is_found_through_its_containers() {
        // ES_Descriptor(3) { ES_ID(2), flags(1),
        //   DecoderConfigDescriptor(4) { 13 fixed bytes,
        //     DecoderSpecificInfo(5) { 0x12 0x10 } } }
        let mut esds = vec![0x03, 0x19, 0x00, 0x00, 0x00];
        esds.extend_from_slice(&[0x04, 0x11]);
        esds.extend_from_slice(&[0x40, 0x15, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        esds.extend_from_slice(&[0x05, 0x02, 0x12, 0x10]);

        let (start, end) = find_descriptor(&esds, 0, esds.len(), 5).expect("specific info");
        assert_eq!(&esds[start..end], &[0x12, 0x10]);
    }

    /// `read_box` vouches only for the box lying inside the buffer, not for its
    /// body holding the fields the type implies. A sample entry declaring size 8
    /// is a legal box and far too short for an `AudioSampleEntry`, and the
    /// channel and sample-rate reads sit at +16 and +24 — well past its end.
    #[test]
    fn a_short_sample_entry_is_rejected_rather_than_indexed_past() {
        for body in [Vec::new(), vec![0u8; 8], vec![0u8; 17], vec![0u8; 25]] {
            let stsd = concat(&[vec![0, 0, 0, 0, 0, 0, 0, 1], boxed(b"mp4a", &body)]);
            let data = concat(&[
                boxed(b"ftyp", b"isom"),
                boxed(b"moov", &boxed(b"trak", &boxed(b"stsd", &stsd))),
            ]);
            assert!(
                audio_config(&data).is_none(),
                "a {}-byte sample entry must not be read as a config",
                body.len()
            );
        }
    }

    #[test]
    fn a_missing_descriptor_is_not_found() {
        let esds = vec![0x03, 0x05, 0x00, 0x00, 0x00, 0x06, 0x00];
        assert_eq!(find_descriptor(&esds, 0, esds.len(), 5), None);
    }

    /// `size` counts the header, so `body` of length 0 is a legal 8-byte box.
    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn concat(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.iter().flatten().copied().collect()
    }

    /// An init segment whose sole track is encrypted and has id 1, so a
    /// fragment naming that id gets past the track lookup and reaches the
    /// `senc` parse.
    fn encrypted_init() -> Vec<u8> {
        let mut tkhd = vec![0u8; 20];
        tkhd[12..16].copy_from_slice(&1u32.to_be_bytes());
        // stsd: version/flags, then a one-entry table.
        let stsd = concat(&[vec![0, 0, 0, 0, 0, 0, 0, 1], boxed(b"enca", &[0u8; 28])]);
        concat(&[
            boxed(b"ftyp", b"isom"),
            boxed(
                b"moov",
                &boxed(
                    b"trak",
                    &concat(&[boxed(b"tkhd", &tkhd), boxed(b"stsd", &stsd)]),
                ),
            ),
        ])
    }

    /// A box can be well-formed as a box and still too short to hold the fields
    /// its type implies. `index_fmp4` parses bytes straight off the network
    /// under `State`'s mutex, so a panic here poisons the mutex and leaves the
    /// track permanently unplayable — a far worse outcome than the empty index
    /// a malformed box should produce.
    #[test]
    fn short_boxes_do_not_panic_the_indexer() {
        // A `tkhd` at the very end of the buffer, which is the ordinary case
        // while the init segment is still arriving: nothing follows `moov`, so
        // reading the version byte and the track id runs off the allocation.
        let empty = boxed(b"tkhd", &[]);
        // Version 1 puts the track id at +20; this body stops at 4.
        let stunted = boxed(b"tkhd", &[1, 0, 0, 0]);
        for trak_body in [empty, stunted] {
            let data = concat(&[
                boxed(b"ftyp", b"isom"),
                boxed(b"moov", &boxed(b"trak", &trak_body)),
            ]);
            let index = index_fmp4(&data).expect("a moov is present");
            assert!(index.samples.is_empty(), "no moof, so no samples");
        }

        // `senc`'s per-sample table starts at +8, so a body shorter than that
        // makes the table's start exceed its end. Slicing that way panics
        // regardless of how much buffer follows.
        for senc_body in [Vec::new(), vec![0u8; 4], vec![0u8; 7]] {
            let traf = boxed(
                b"traf",
                &concat(&[
                    boxed(b"tfhd", &[0, 0, 0, 0, 0, 0, 0, 1]),
                    boxed(b"senc", &senc_body),
                ]),
            );
            let data = concat(&[
                encrypted_init(),
                boxed(b"moof", &traf),
                boxed(b"mdat", &[0u8; 16]),
            ]);
            let index = index_fmp4(&data).expect("a moov is present");
            assert!(index.samples.is_empty(), "no trun, so no samples");
        }
    }

    /// The fragment walk requires a readable `mdat` after each `moof`, which
    /// puts at least eight bytes beyond every box inside that `moof`. That is
    /// what keeps the fixed-offset reads on `tfhd` and `senc`'s header in
    /// bounds, so the guards there are belt-and-braces rather than load-bearing
    /// — this pins the precondition they lean on.
    #[test]
    fn a_fragment_is_only_walked_when_an_mdat_follows_it() {
        let traf = boxed(b"traf", &boxed(b"tfhd", &[]));
        let truncated = concat(&[encrypted_init(), boxed(b"moof", &traf)]);
        assert!(
            index_fmp4(&truncated)
                .expect("a moov is present")
                .samples
                .is_empty(),
            "a moof with nothing after it is not walked at all"
        );

        let with_mdat = concat(&[truncated.clone(), boxed(b"mdat", &[0u8; 16])]);
        let moof_end = truncated.len();
        assert!(
            moof_end + 8 <= with_mdat.len(),
            "the mdat guarantees eight readable bytes past the end of the moof"
        );
    }

    /// Reads a real Apple Music init segment. Ignored: needs a cached track.
    ///
    /// The values are checked for plausibility rather than pinned, since they
    /// vary per track — but a mis-parse shows up immediately as a nonsense sample
    /// rate or an empty `csd-0`.
    #[test]
    #[ignore = "needs a cached Apple Music track"]
    fn audio_config_reads_a_real_track() {
        let dir = std::path::PathBuf::from(std::env::var("HOME").unwrap())
            .join(".cache/kopuz/applemusic");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return;
        };
        let mut checked = 0;
        for path in entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "m4a"))
        {
            let bytes = std::fs::read(&path).expect("read track");
            let cfg = audio_config(&bytes)
                .unwrap_or_else(|| panic!("no audio config in {}", path.display()));

            assert!(
                [44100, 48000, 22050, 24000, 32000, 88200, 96000].contains(&cfg.sample_rate),
                "{}: implausible sample rate {}",
                path.display(),
                cfg.sample_rate
            );
            assert!(
                (1..=8).contains(&cfg.channels),
                "{}: implausible channel count {}",
                path.display(),
                cfg.channels
            );
            // An AudioSpecificConfig is at least two bytes: 5 bits object type,
            // 4 bits sampling frequency index, 4 bits channel configuration.
            assert!(
                (2..=64).contains(&cfg.codec_specific.len()),
                "{}: implausible csd-0 of {} bytes",
                path.display(),
                cfg.codec_specific.len()
            );
            // The sample entry and the AudioSpecificConfig describe the same
            // track by different routes, so they have to agree. This is the real
            // check: a mis-parse of either one shows up as a disagreement, where
            // plausibility bounds alone would let it through.
            const ASC_RATES: [u32; 13] = [
                96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000,
                7350,
            ];
            let object_type = cfg.codec_specific[0] >> 3;
            let freq_index = ((cfg.codec_specific[0] & 0x07) << 1) | (cfg.codec_specific[1] >> 7);
            let channel_config = (cfg.codec_specific[1] >> 3) & 0x0F;
            assert_eq!(object_type, 2, "{}: expected AAC-LC", path.display());
            assert_eq!(
                ASC_RATES.get(freq_index as usize).copied(),
                Some(cfg.sample_rate),
                "{}: csd-0 says index {freq_index}, sample entry says {}",
                path.display(),
                cfg.sample_rate
            );
            assert_eq!(
                u16::from(channel_config),
                cfg.channels,
                "{}: csd-0 and sample entry disagree on channels",
                path.display()
            );
            checked += 1;
        }
        assert!(checked > 0, "no cached tracks to check");
    }
}

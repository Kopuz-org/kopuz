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

fn find_all_children(
    data: &[u8],
    body_start: usize,
    body_end: usize,
) -> Vec<(usize, usize, usize)> {
    let mut result = Vec::new();
    let mut pos = body_start;
    while pos < body_end {
        let (bs, _be, total) = match read_box(data, pos) {
            Some(v) => v,
            None => break,
        };
        result.push((pos, bs, total));
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

    for (trak_box_start, trak_body_start, trak_total) in
        find_all_children(data, moov_body_start, moov_body_end)
    {
        let trak_body_end = trak_body_start + trak_total - 8;
        if box_type(data, trak_box_start) != TRAK {
            continue;
        }

        let track_id = find_child(data, trak_body_start, trak_body_end, TKHD)
            .map(|(s, _, _)| {
                let version = data[s];
                let offset = if version == 0 { 12 } else { 20 };
                u32be(data, s + offset)
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

        for (traf_pos, traf_bs, traf_total) in find_all_children(data, moof_bs, moof_be) {
            if box_type(data, traf_pos) != TRAF {
                continue;
            }
            let traf_be = traf_bs + traf_total - 8;

            let tfhd = find_child(data, traf_bs, traf_be, TFHD);
            let track_id = tfhd
                .as_ref()
                .map(|(s, _, _)| u32be(data, s + 4))
                .unwrap_or(0);

            let Some(ti) = track_infos.iter().find(|t| t.track_id == track_id) else {
                continue;
            };
            let per_sample_iv_size = ti.default_iv_size;

            let mut traf_ivs: Vec<[u8; 16]> = Vec::new();
            let mut traf_subs: Vec<Vec<(u16, u32)>> = Vec::new();
            if let Some((senc_bs, senc_be, _)) = find_child(data, traf_bs, traf_be, SENC) {
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

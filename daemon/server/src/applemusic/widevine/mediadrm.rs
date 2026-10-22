//! Widevine on Android
use super::super::cenc::EncryptedSample;

/// `MediaDrm.KEY_TYPE_STREAMING`.
pub const KEY_TYPE_STREAMING: i32 = 1;
/// `MediaCodec.CRYPTO_MODE_AES_CTR`
pub const CRYPTO_MODE_AES_CTR: i32 = 1;
pub const AUDIO_MIME: &str = "audio/mp4";
pub const SECURITY_LEVEL_PROPERTY: &str = "securityLevel";
pub const SECURITY_LEVEL_L3: &str = "L3";

/// Widevine's scheme id split the way `java.util.UUID(long, long)` takes it.
///
/// Derived from [`WIDEVINE_SYSTEM_ID`](super::WIDEVINE_SYSTEM_ID) rather than
/// written out again, so the Android UUID and the `pssh` box can't drift apart.
pub const fn widevine_uuid_halves() -> (i64, i64) {
    let id = super::WIDEVINE_SYSTEM_ID;
    let mut most = 0u64;
    let mut least = 0u64;
    let mut i = 0;
    while i < 8 {
        most = (most << 8) | id[i] as u64;
        i += 1;
    }
    while i < 16 {
        least = (least << 8) | id[i] as u64;
        i += 1;
    }
    (most as i64, least as i64)
}

/// One sample's crypto description, shaped for `MediaCodec.CryptoInfo.set`.
///
/// `key` there is the key *id*, not the key: the key itself stays inside the DRM
/// session and is never visible to us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SampleCryptoInfo {
    /// Zero-padded to 16 bytes, as CENC's 8-byte IVs must be for AES-CTR.
    pub iv: [u8; 16],
    /// Clear byte count per subsample. `int[]` on the Java side.
    pub clear_bytes: Vec<i32>,
    /// Encrypted byte count per subsample, pairwise with `clear_bytes`.
    pub encrypted_bytes: Vec<i32>,
}

impl SampleCryptoInfo {
    pub fn subsample_count(&self) -> i32 {
        self.clear_bytes.len() as i32
    }
}

/// Describe `sample` for `MediaCodec`.
///
/// A `senc` box may leave the subsample list empty, which means the whole sample
/// is encrypted. `CryptoInfo` has no way to say that, so it becomes the single
/// subsample "no clear bytes, all of it encrypted" — the same thing the desktop
/// path expresses by passing an empty subsample list to the CDM.
pub fn sample_crypto_info(sample: &EncryptedSample) -> SampleCryptoInfo {
    if sample.subs.is_empty() {
        return SampleCryptoInfo {
            iv: sample.iv,
            clear_bytes: vec![0],
            encrypted_bytes: vec![sample.len as i32],
        };
    }
    SampleCryptoInfo {
        iv: sample.iv,
        clear_bytes: sample.subs.iter().map(|(clear, _)| *clear as i32).collect(),
        encrypted_bytes: sample
            .subs
            .iter()
            .map(|(_, encrypted)| *encrypted as i32)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The UUID and the `pssh` box must name the same scheme. A byte-order slip
    /// in the derivation would produce a UUID no DRM plugin answers to, and on
    /// Android that surfaces as an unhelpful "unsupported scheme".
    #[test]
    fn the_uuid_halves_rebuild_the_system_id() {
        let (most, least) = widevine_uuid_halves();
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&(most as u64).to_be_bytes());
        bytes[8..].copy_from_slice(&(least as u64).to_be_bytes());
        assert_eq!(bytes, super::super::WIDEVINE_SYSTEM_ID);
    }

    /// Pinned so the constant is checked against the published UUID rather than
    /// only against itself.
    #[test]
    fn the_uuid_is_widevines() {
        let (most, least) = widevine_uuid_halves();
        // edef8ba9-79d6-4ace-a3c8-27dcd51d21ed
        assert_eq!(most as u64, 0xedef_8ba9_79d6_4ace);
        assert_eq!(least as u64, 0xa3c8_27dc_d51d_21ed);
    }

    fn sample(len: usize, subs: Vec<(u16, u32)>) -> EncryptedSample {
        EncryptedSample {
            start: 1000,
            len,
            iv: [0xAB; 16],
            subs,
        }
    }

    /// An empty subsample list means "all encrypted", which `CryptoInfo` can
    /// only express as one subsample with zero clear bytes. Passing zero
    /// subsamples instead would tell the codec the sample is entirely cleartext.
    #[test]
    fn a_sample_with_no_subsamples_is_wholly_encrypted() {
        let info = sample_crypto_info(&sample(800, Vec::new()));
        assert_eq!(info.subsample_count(), 1);
        assert_eq!(info.clear_bytes, vec![0]);
        assert_eq!(info.encrypted_bytes, vec![800]);
        assert_eq!(info.iv, [0xAB; 16]);
    }

    #[test]
    fn subsamples_are_carried_across_pairwise() {
        let info = sample_crypto_info(&sample(100, vec![(16, 32), (0, 48), (4, 0)]));
        assert_eq!(info.subsample_count(), 3);
        assert_eq!(info.clear_bytes, vec![16, 0, 4]);
        assert_eq!(info.encrypted_bytes, vec![32, 48, 0]);
    }

    /// The two arrays are read in lockstep by the codec, so a mismatch in length
    /// would have it read past the end of one of them.
    #[test]
    fn the_two_arrays_always_match_in_length() {
        for subs in [
            Vec::new(),
            vec![(0, 10)],
            vec![(1, 2), (3, 4)],
            vec![(0, 0); 40],
        ] {
            let info = sample_crypto_info(&sample(500, subs));
            assert_eq!(info.clear_bytes.len(), info.encrypted_bytes.len());
            assert!(
                info.subsample_count() >= 1,
                "a sample has at least one part"
            );
        }
    }

    /// Subsample sizes have to account for the whole sample; if they didn't, the
    /// codec would be handed a length that disagrees with the buffer.
    #[test]
    fn subsample_sizes_cover_the_sample() {
        let info = sample_crypto_info(&sample(100, vec![(16, 32), (0, 48), (4, 0)]));
        let total: i32 = info
            .clear_bytes
            .iter()
            .chain(info.encrypted_bytes.iter())
            .sum();
        assert_eq!(total, 100);
    }
}

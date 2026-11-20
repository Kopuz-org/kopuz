//! Widevine through the NDK: `AMediaDrm` for the licence, and an `AMediaCodec`
//! wired to an `AMediaCrypto` that decrypts and decodes each sample together.
//!
//! The C API rather than `android.media` over JNI, so nothing here needs a Java
//! class loader or a thread attached to the VM.

use std::ffi::{CStr, c_void};
use std::ptr::{NonNull, null, null_mut};
use std::time::{Duration, Instant};

use ndk_sys as sys;

use super::super::cenc::{AudioConfig, EncryptedSample};
use super::super::pcm::{AacDecoder, PcmFormat};
use super::WIDEVINE_SYSTEM_ID;
use super::mediadrm::sample_crypto_info;

const AAC_MIME: &CStr = c"audio/mp4a-latm";
const KEY_MIME: &CStr = c"audio/mp4";

/// `AudioFormat.ENCODING_PCM_16BIT`, what an AAC decoder emits unless asked for float.
const PCM_16BIT: i32 = 2;

/// How long one dequeue call blocks. Short, so a cancelled track is noticed quickly.
const DEQUEUE_TIMEOUT_US: i64 = 10_000;

/// How long the codec may refuse input or withhold output before it counts as hung.
const CODEC_STALL: Duration = Duration::from_secs(5);

fn check(status: sys::media_status_t, what: &str) -> Result<(), String> {
    if status == sys::media_status_t::AMEDIA_OK {
        Ok(())
    } else {
        Err(format!("{what} failed (media status {})", status.0))
    }
}

/// An open `MediaDrm` session holding one track's content keys.
pub struct DrmSession {
    drm: NonNull<sys::AMediaDrm>,
    session: sys::AMediaDrmSessionId,
}

/// Safety: `AMediaDrm` is a binder proxy whose calls are internally serialised,
/// and this type is only ever used by one thread at a time: it is created on a
/// blocking task, used across an await on the licence request, then moved into
/// the decode thread for good.
unsafe impl Send for DrmSession {}

impl DrmSession {
    /// Open a Widevine session, asking for software (L3) security first.
    ///
    /// L3 keys may be used by the ordinary software AAC decoder. A device that
    /// refuses the request keeps its default level, which is logged so a licence
    /// rejection can be traced back to it.
    pub fn open() -> Result<Self, String> {
        let drm = unsafe { sys::AMediaDrm_createByUUID(WIDEVINE_SYSTEM_ID.as_ptr()) };
        let drm = NonNull::new(drm).ok_or("this device has no Widevine DRM")?;

        let level = unsafe {
            sys::AMediaDrm_setPropertyString(
                drm.as_ptr(),
                c"securityLevel".as_ptr(),
                c"L3".as_ptr(),
            )
        };
        if level != sys::media_status_t::AMEDIA_OK {
            tracing::warn!(
                status = level.0,
                "am.mediadrm: could not ask for L3, keeping the device default"
            );
        }

        let mut session = sys::AMediaDrmSessionId {
            ptr: null(),
            length: 0,
        };
        let opened = unsafe { sys::AMediaDrm_openSession(drm.as_ptr(), &mut session) };
        if let Err(e) = check(opened, "opening a MediaDrm session") {
            unsafe { sys::AMediaDrm_release(drm.as_ptr()) };
            return Err(e);
        }

        let this = Self { drm, session };
        tracing::info!(
            level = this.property(c"securityLevel").as_deref().unwrap_or("?"),
            version = this.property(c"version").as_deref().unwrap_or("?"),
            "am.mediadrm: session open"
        );
        Ok(this)
    }

    fn property(&self, name: &CStr) -> Option<String> {
        let mut value = null();
        let status = unsafe {
            sys::AMediaDrm_getPropertyString(self.drm.as_ptr(), name.as_ptr(), &mut value)
        };
        if status != sys::media_status_t::AMEDIA_OK || value.is_null() {
            return None;
        }
        Some(
            unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned(),
        )
    }

    /// The licence challenge for `pssh_box`.
    ///
    /// Copied out at once: the buffer belongs to the DRM object and the next call
    /// overwrites it.
    pub fn key_request(&self, pssh_box: &[u8]) -> Result<Vec<u8>, String> {
        let mut request = null();
        let mut request_len = 0usize;
        let status = unsafe {
            sys::AMediaDrm_getKeyRequest(
                self.drm.as_ptr(),
                &self.session,
                pssh_box.as_ptr(),
                pssh_box.len(),
                KEY_MIME.as_ptr(),
                sys::AMediaDrmKeyType::KEY_TYPE_STREAMING,
                null(),
                0,
                &mut request,
                &mut request_len,
            )
        };
        check(status, "building the licence challenge")?;
        if request.is_null() || request_len == 0 {
            return Err("MediaDrm returned an empty licence challenge".to_string());
        }
        Ok(unsafe { std::slice::from_raw_parts(request, request_len) }.to_vec())
    }

    /// Load Apple's licence into the session.
    pub fn provide_key_response(&self, licence: &[u8]) -> Result<(), String> {
        let mut key_set = sys::AMediaDrmKeySetId {
            ptr: null(),
            length: 0,
        };
        let status = unsafe {
            sys::AMediaDrm_provideKeyResponse(
                self.drm.as_ptr(),
                &self.session,
                licence.as_ptr(),
                licence.len(),
                &mut key_set,
            )
        };
        check(status, "loading the licence")
    }

    fn crypto(&self) -> Result<NonNull<sys::AMediaCrypto>, String> {
        let mut uuid = WIDEVINE_SYSTEM_ID;
        let crypto = unsafe {
            sys::AMediaCrypto_new(
                uuid.as_mut_ptr(),
                self.session.ptr.cast::<c_void>(),
                self.session.length,
            )
        };
        NonNull::new(crypto)
            .ok_or_else(|| "creating MediaCrypto for the session failed".to_string())
    }
}

impl Drop for DrmSession {
    fn drop(&mut self) {
        unsafe {
            let closed = sys::AMediaDrm_closeSession(self.drm.as_ptr(), &self.session);
            if closed != sys::media_status_t::AMEDIA_OK {
                tracing::debug!(status = closed.0, "am.mediadrm: closing the session failed");
            }
            sys::AMediaDrm_release(self.drm.as_ptr());
        }
    }
}

/// An AAC decoder that takes Widevine-encrypted samples.
pub struct MediaCodecAac {
    codec: NonNull<sys::AMediaCodec>,
    crypto: NonNull<sys::AMediaCrypto>,
    key_id: [u8; 16],
    input: AudioConfig,
    format: Option<PcmFormat>,
    output_ended: bool,
    /// Declared last so it is released after the codec and crypto that use its
    /// session, which [`Drop`] tears down first.
    _drm: DrmSession,
}

/// Safety: `AMediaCodec` may be driven from any thread as long as calls are not
/// concurrent, and this type is owned by the single decode thread.
unsafe impl Send for MediaCodecAac {}

impl MediaCodecAac {
    pub fn new(drm: DrmSession, key_id: &[u8], input: &AudioConfig) -> Result<Self, String> {
        let key_id: [u8; 16] = key_id
            .try_into()
            .map_err(|_| format!("a Widevine key id is 16 bytes, not {}", key_id.len()))?;

        let crypto = drm.crypto()?;
        if unsafe { sys::AMediaCrypto_requiresSecureDecoderComponent(AAC_MIME.as_ptr()) } {
            tracing::warn!(
                "am.mediacodec: the DRM asks for a secure AAC decoder; trying the plain one"
            );
        }

        let Some(codec) =
            NonNull::new(unsafe { sys::AMediaCodec_createDecoderByType(AAC_MIME.as_ptr()) })
        else {
            unsafe { sys::AMediaCrypto_delete(crypto.as_ptr()) };
            return Err("this device has no AAC decoder".to_string());
        };

        let this = Self {
            codec,
            crypto,
            key_id,
            input: input.clone(),
            format: None,
            output_ended: false,
            _drm: drm,
        };
        this.configure()?;
        Ok(this)
    }

    fn configure(&self) -> Result<(), String> {
        unsafe {
            let format = sys::AMediaFormat_new();
            if format.is_null() {
                return Err("allocating a MediaFormat failed".to_string());
            }
            sys::AMediaFormat_setString(format, c"mime".as_ptr(), AAC_MIME.as_ptr());
            sys::AMediaFormat_setInt32(
                format,
                c"sample-rate".as_ptr(),
                self.input.sample_rate as i32,
            );
            sys::AMediaFormat_setInt32(
                format,
                c"channel-count".as_ptr(),
                i32::from(self.input.channels),
            );
            sys::AMediaFormat_setInt32(format, c"is-adts".as_ptr(), 0);
            sys::AMediaFormat_setBuffer(
                format,
                c"csd-0".as_ptr(),
                self.input.codec_specific.as_ptr().cast::<c_void>(),
                self.input.codec_specific.len(),
            );

            let configured = sys::AMediaCodec_configure(
                self.codec.as_ptr(),
                format,
                null_mut(),
                self.crypto.as_ptr(),
                0,
            );
            sys::AMediaFormat_delete(format);
            check(configured, "configuring the AAC decoder")?;
            check(
                sys::AMediaCodec_start(self.codec.as_ptr()),
                "starting the AAC decoder",
            )
        }
    }

    /// Wait for a free input buffer, draining output meanwhile so the codec
    /// can't deadlock on a full output queue.
    fn input_slot(&mut self, out: &mut Vec<u8>) -> Result<usize, String> {
        let deadline = Instant::now() + CODEC_STALL;
        loop {
            let index = unsafe {
                sys::AMediaCodec_dequeueInputBuffer(self.codec.as_ptr(), DEQUEUE_TIMEOUT_US)
            };
            if index >= 0 {
                return Ok(index as usize);
            }
            self.drain(out, 0)?;
            if Instant::now() >= deadline {
                return Err("the AAC decoder stopped accepting input".to_string());
            }
        }
    }

    /// Collect every output buffer that is ready, waiting up to `timeout_us` for
    /// the first.
    fn drain(&mut self, out: &mut Vec<u8>, mut timeout_us: i64) -> Result<(), String> {
        loop {
            let mut info = sys::AMediaCodecBufferInfo {
                offset: 0,
                size: 0,
                presentationTimeUs: 0,
                flags: 0,
            };
            let index = unsafe {
                sys::AMediaCodec_dequeueOutputBuffer(self.codec.as_ptr(), &mut info, timeout_us)
            };
            timeout_us = 0;

            if index >= 0 {
                self.take_output(index as usize, &info, out)?;
                if info.flags & sys::AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM != 0 {
                    self.output_ended = true;
                    return Ok(());
                }
                continue;
            }
            match index as i32 {
                sys::AMEDIACODEC_INFO_TRY_AGAIN_LATER => return Ok(()),
                sys::AMEDIACODEC_INFO_OUTPUT_FORMAT_CHANGED => self.read_output_format()?,
                sys::AMEDIACODEC_INFO_OUTPUT_BUFFERS_CHANGED => {}
                other => return Err(format!("the AAC decoder failed (dequeue status {other})")),
            }
        }
    }

    fn take_output(
        &mut self,
        index: usize,
        info: &sys::AMediaCodecBufferInfo,
        out: &mut Vec<u8>,
    ) -> Result<(), String> {
        if self.format.is_none() {
            self.format = Some(PcmFormat {
                sample_rate: self.input.sample_rate,
                channels: self.input.channels,
            });
        }
        let mut capacity = 0usize;
        let buffer =
            unsafe { sys::AMediaCodec_getOutputBuffer(self.codec.as_ptr(), index, &mut capacity) };
        let (offset, size) = (info.offset.max(0) as usize, info.size.max(0) as usize);
        if !buffer.is_null() && size > 0 && offset + size <= capacity {
            out.extend_from_slice(unsafe { std::slice::from_raw_parts(buffer.add(offset), size) });
        }
        check(
            unsafe { sys::AMediaCodec_releaseOutputBuffer(self.codec.as_ptr(), index, false) },
            "releasing a decoded buffer",
        )
    }

    fn read_output_format(&mut self) -> Result<(), String> {
        unsafe {
            let format = sys::AMediaCodec_getOutputFormat(self.codec.as_ptr());
            if format.is_null() {
                return Ok(());
            }
            let mut rate = 0i32;
            let mut channels = 0i32;
            let mut encoding = PCM_16BIT;
            let has_rate = sys::AMediaFormat_getInt32(format, c"sample-rate".as_ptr(), &mut rate);
            let has_channels =
                sys::AMediaFormat_getInt32(format, c"channel-count".as_ptr(), &mut channels);
            sys::AMediaFormat_getInt32(format, c"pcm-encoding".as_ptr(), &mut encoding);
            sys::AMediaFormat_delete(format);

            if encoding != PCM_16BIT {
                return Err(format!(
                    "the AAC decoder emits PCM encoding {encoding}, not 16-bit"
                ));
            }
            if has_rate && has_channels && rate > 0 && channels > 0 {
                let format = PcmFormat {
                    sample_rate: rate as u32,
                    channels: channels as u16,
                };
                if self.format.is_some_and(|known| known != format) {
                    return Err(format!(
                        "the decoder changed its output to {format:?} mid-track"
                    ));
                }
                tracing::debug!(?format, "am.mediacodec: output format");
                self.format = Some(format);
            }
        }
        Ok(())
    }
}

impl AacDecoder for MediaCodecAac {
    fn decode(
        &mut self,
        sample: &EncryptedSample,
        data: &[u8],
        pts_us: u64,
    ) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let slot = self.input_slot(&mut out)?;

        let mut capacity = 0usize;
        let buffer =
            unsafe { sys::AMediaCodec_getInputBuffer(self.codec.as_ptr(), slot, &mut capacity) };
        if buffer.is_null() || data.len() > capacity {
            return Err(format!(
                "a {}-byte sample doesn't fit the decoder's {capacity}-byte buffer",
                data.len()
            ));
        }
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), buffer, data.len()) };

        let crypto = sample_crypto_info(sample);
        let mut clear: Vec<usize> = crypto.clear_bytes.iter().map(|&n| n as usize).collect();
        let mut encrypted: Vec<usize> =
            crypto.encrypted_bytes.iter().map(|&n| n as usize).collect();
        let mut key = self.key_id;
        let mut iv = crypto.iv;
        let info = unsafe {
            sys::AMediaCodecCryptoInfo_new(
                crypto.subsample_count(),
                key.as_mut_ptr(),
                iv.as_mut_ptr(),
                sys::cryptoinfo_mode_t::AMEDIACODECRYPTOINFO_MODE_AES_CTR,
                clear.as_mut_ptr(),
                encrypted.as_mut_ptr(),
            )
        };
        if info.is_null() {
            return Err("allocating the sample's CryptoInfo failed".to_string());
        }
        let queued = unsafe {
            sys::AMediaCodec_queueSecureInputBuffer(self.codec.as_ptr(), slot, 0, info, pts_us, 0)
        };
        unsafe { sys::AMediaCodecCryptoInfo_delete(info) };
        check(queued, "decrypting a sample")?;

        self.drain(&mut out, 0)?;
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let slot = self.input_slot(&mut out)?;
        check(
            unsafe {
                sys::AMediaCodec_queueInputBuffer(
                    self.codec.as_ptr(),
                    slot,
                    0,
                    0,
                    0,
                    sys::AMEDIACODEC_BUFFER_FLAG_END_OF_STREAM,
                )
            },
            "ending the stream",
        )?;

        let deadline = Instant::now() + CODEC_STALL;
        while !self.output_ended {
            self.drain(&mut out, DEQUEUE_TIMEOUT_US)?;
            if Instant::now() >= deadline {
                tracing::warn!("am.mediacodec: the decoder never signalled the end of stream");
                break;
            }
        }
        Ok(out)
    }

    fn format(&self) -> Option<PcmFormat> {
        self.format
    }
}

impl Drop for MediaCodecAac {
    fn drop(&mut self) {
        unsafe {
            sys::AMediaCodec_stop(self.codec.as_ptr());
            sys::AMediaCodec_delete(self.codec.as_ptr());
            sys::AMediaCrypto_delete(self.crypto.as_ptr());
        }
    }
}

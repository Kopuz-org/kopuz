//! Minimal bindings to the public libcdio/paranoia ABI. Libraries are optional
//! at runtime so systems without optical drives do not acquire a hard dependency.

use std::cell::Cell;
use std::ffi::{CStr, CString, c_char, c_int, c_long, c_void};
use std::io;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use libloading::Library;

use super::device::Readiness;
use super::{Disc, DiscTrack, SECTOR_BYTES, SectorReader};

type Handle = *mut c_void;
type Callback = Option<unsafe extern "C" fn(c_long, c_int)>;

struct Bindings {
    open: unsafe extern "C" fn(*const c_char, c_int) -> Handle,
    destroy: unsafe extern "C" fn(Handle),
    devices: unsafe extern "C" fn(c_int) -> *mut *mut c_char,
    free_devices: unsafe extern "C" fn(*mut *mut c_char),
    ready: unsafe extern "C" fn(Handle, u32) -> c_int,
    last_sense: unsafe extern "C" fn(Handle, *mut *mut u8) -> c_int,
    free_memory: unsafe extern "C" fn(*mut c_void),
    first_track: unsafe extern "C" fn(Handle) -> u8,
    last_track: unsafe extern "C" fn(Handle) -> u8,
    track_start: unsafe extern "C" fn(Handle, u8) -> i32,
    track_format: unsafe extern "C" fn(Handle, u8) -> c_int,
    media_changed: unsafe extern "C" fn(Handle) -> c_int,
    identify: unsafe extern "C" fn(Handle, c_int, *mut *mut c_char) -> Handle,
    cdda_open: unsafe extern "C" fn(Handle) -> c_int,
    cdda_close: unsafe extern "C" fn(Handle) -> bool,
    audio_end: unsafe extern "C" fn(Handle) -> i32,
    channels: unsafe extern "C" fn(Handle, u8) -> c_int,
    preemphasis: unsafe extern "C" fn(Handle, u8) -> c_int,
    init: unsafe extern "C" fn(Handle) -> Handle,
    free: unsafe extern "C" fn(Handle),
    mode: unsafe extern "C" fn(Handle, c_int),
    range: unsafe extern "C" fn(Handle, c_long, c_long),
    seek: unsafe extern "C" fn(Handle, i32, c_int) -> i32,
    read: unsafe extern "C" fn(Handle, Callback, c_int) -> *const i16,
    // Symbols are copied, so retain the owning libraries until all handles close.
    _paranoia: Library,
    _cdda: Library,
    _cdio: Library,
}

fn library(names: &[&str]) -> io::Result<Library> {
    let mut directories = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        directories.push(parent.to_path_buf());
    }
    if cfg!(target_os = "macos") {
        directories.extend([
            PathBuf::from("/opt/homebrew/lib"),
            PathBuf::from("/usr/local/lib"),
        ]);
    }
    for path in directories
        .iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .chain(names.iter().map(PathBuf::from))
    {
        // SAFETY: only known system libraries are loaded; callers verify every symbol's ABI.
        if let Ok(library) = unsafe { Library::new(path) } {
            return Ok(library);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "Audio CD support requires libcdio and libcdio-paranoia. Install them alongside Kopuz or through your system package manager.",
    ))
}

impl Bindings {
    fn load() -> io::Result<Arc<Self>> {
        let cdio = library(&[
            "libcdio.so.19",
            "libcdio.so",
            "libcdio.19.dylib",
            "libcdio.dylib",
            "libcdio-19.dll",
        ])?;
        let cdda = library(&[
            "libcdio_cdda.so.2",
            "libcdio_cdda.so",
            "libcdio_cdda.2.dylib",
            "libcdio_cdda.dylib",
            "libcdio_cdda-2.dll",
        ])?;
        let paranoia = library(&[
            "libcdio_paranoia.so.2",
            "libcdio_paranoia.so",
            "libcdio_paranoia.2.dylib",
            "libcdio_paranoia.dylib",
            "libcdio_paranoia-2.dll",
        ])?;
        let err = |error: libloading::Error| {
            io::Error::other(format!("Incompatible audio CD library: {error}"))
        };
        // SAFETY: signatures match libcdio device.h/track.h and paranoia cdda.h/paranoia.h.
        // Opaque pointers never escape Drive, and libraries outlive their copied symbols.
        unsafe {
            Ok(Arc::new(Self {
                open: *cdio.get(b"cdio_open\0").map_err(err)?,
                destroy: *cdio.get(b"cdio_destroy\0").map_err(err)?,
                devices: *cdio.get(b"cdio_get_devices\0").map_err(err)?,
                free_devices: *cdio.get(b"cdio_free_device_list\0").map_err(err)?,
                ready: *cdio.get(b"mmc_test_unit_ready\0").map_err(err)?,
                last_sense: *cdio.get(b"mmc_last_cmd_sense\0").map_err(err)?,
                free_memory: *cdio.get(b"cdio_free\0").map_err(err)?,
                first_track: *cdio.get(b"cdio_get_first_track_num\0").map_err(err)?,
                last_track: *cdio.get(b"cdio_get_last_track_num\0").map_err(err)?,
                track_start: *cdio.get(b"cdio_get_track_lsn\0").map_err(err)?,
                track_format: *cdio.get(b"cdio_get_track_format\0").map_err(err)?,
                media_changed: *cdio.get(b"cdio_get_media_changed\0").map_err(err)?,
                identify: *cdda.get(b"cdio_cddap_identify_cdio\0").map_err(err)?,
                cdda_open: *cdda.get(b"cdio_cddap_open\0").map_err(err)?,
                cdda_close: *cdda.get(b"cdio_cddap_close_no_free_cdio\0").map_err(err)?,
                audio_end: *cdda.get(b"cdio_cddap_disc_lastsector\0").map_err(err)?,
                channels: *cdda.get(b"cdio_cddap_track_channels\0").map_err(err)?,
                preemphasis: *cdda.get(b"cdio_cddap_track_preemp\0").map_err(err)?,
                init: *paranoia.get(b"cdio_paranoia_init\0").map_err(err)?,
                free: *paranoia.get(b"cdio_paranoia_free\0").map_err(err)?,
                mode: *paranoia.get(b"cdio_paranoia_modeset\0").map_err(err)?,
                range: *paranoia.get(b"cdio_paranoia_set_range\0").map_err(err)?,
                seek: *paranoia.get(b"cdio_paranoia_seek\0").map_err(err)?,
                read: *paranoia.get(b"cdio_paranoia_read_limited\0").map_err(err)?,
                _paranoia: paranoia,
                _cdda: cdda,
                _cdio: cdio,
            }))
        }
    }
}

pub(super) struct Drive {
    api: Arc<Bindings>,
    cdio: NonNull<c_void>,
    cdda: Option<NonNull<c_void>>,
    paranoia: Option<NonNull<c_void>>,
    next_sector: Option<i32>,
    checked: std::time::Instant,
}

#[derive(Clone)]
pub(crate) struct Reader {
    device: String,
    shared: Arc<ReaderState>,
    #[cfg(test)]
    driver: c_int,
}

#[derive(Default)]
struct ReaderState {
    generation: AtomicU64,
    drive: Mutex<Option<CachedDrive>>,
}

struct CachedDrive {
    drive: Drive,
    disc: String,
    node: Option<super::device::Node>,
    failed: bool,
}

pub(crate) struct ReadRequest {
    reader: Reader,
    generation: u64,
}

impl Reader {
    pub(crate) fn new(device: &str) -> Self {
        Self {
            device: device.to_string(),
            shared: Arc::new(ReaderState::default()),
            #[cfg(test)]
            driver: 11,
        }
    }

    pub(crate) fn request(&self) -> ReadRequest {
        // Reserve before spawning blocking work: an older queued request must
        // not take the drive back if the listener quickly skips several tracks.
        let generation = self
            .shared
            .generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        ReadRequest {
            reader: self.clone(),
            generation,
        }
    }
}

impl ReadRequest {
    fn check_current(&self) -> io::Result<()> {
        if self.generation != self.reader.shared.generation.load(Ordering::Relaxed) {
            return Err(io::Error::other("CD track reader was superseded"));
        }
        Ok(())
    }

    pub(crate) fn open(self, key: &str) -> io::Result<Box<dyn symphonia::core::io::MediaSource>> {
        let started = std::time::Instant::now();
        let mut state = self
            .reader
            .shared
            .drive
            .lock()
            .map_err(|_| io::Error::other("CD reader lock poisoned"))?;
        self.check_current()?;
        #[cfg(test)]
        let mut probe = Drive::open_driver(&self.reader.device, self.reader.driver)?;
        #[cfg(not(test))]
        let mut probe = Drive::open(&self.reader.device)?;
        // A fresh TOC validates disc identity; libcdio caches it on an existing handle.
        let disc = probe.disc()?;
        let track = disc.track(key)?.clone();
        let node = super::device::Node::read(&self.reader.device).ok();
        let reuse = state
            .as_ref()
            .is_some_and(|cached| !cached.failed && cached.disc == disc.id && cached.node == node);
        if !reuse {
            *state = Some(CachedDrive {
                drive: probe,
                disc: disc.id,
                node,
                failed: false,
            });
        }
        let cached = state
            .as_mut()
            .ok_or_else(|| io::Error::other("CD reader unavailable"))?;
        if let Err(error) = cached.drive.prepare(&track) {
            cached.failed = true;
            return Err(error);
        }
        self.check_current()?;
        drop(state);
        tracing::debug!(
            reused = reuse,
            elapsed_ms = started.elapsed().as_millis(),
            track = track.number,
            "CD track reader prepared"
        );
        Ok(Box::new(super::WavStream::new(
            super::buffer::ReadAhead::new(self, &track)?,
            track,
        )?))
    }
}

impl SectorReader for ReadRequest {
    fn sector(&mut self, lsn: i32, output: &mut [u8; SECTOR_BYTES]) -> io::Result<()> {
        let mut state = self
            .reader
            .shared
            .drive
            .lock()
            .map_err(|_| io::Error::other("CD reader lock poisoned"))?;
        self.check_current()?;
        let cached = state
            .as_mut()
            .ok_or_else(|| io::Error::other("CD reader unavailable"))?;
        let result = cached.drive.read_sector(lsn, output);
        if result.is_err() {
            cached.failed = true;
        }
        result
    }
}

// SAFETY: these C handles have no thread affinity. Access requires &mut Drive;
// the stream wraps it in a Mutex, so no handle is ever used concurrently.
unsafe impl Send for Drive {}

impl Drive {
    fn readiness(&mut self) -> Readiness {
        // SAFETY: the handle is exclusively owned. libcdio allocates the sense
        // bytes and reports their length; release them using its own allocator.
        unsafe {
            if (self.api.ready)(self.cdio.as_ptr(), 1000) == 0 {
                return Readiness::Ready;
            }
            let mut sense = std::ptr::null_mut();
            let length = (self.api.last_sense)(self.cdio.as_ptr(), &mut sense);
            if sense.is_null() {
                return Readiness::Unknown;
            }
            let absent = length > 0
                && sense_reports_absent(std::slice::from_raw_parts(sense, length as usize));
            (self.api.free_memory)(sense.cast());
            if absent {
                Readiness::Absent
            } else {
                Readiness::Unknown
            }
        }
    }

    pub(super) fn open(device: &str) -> io::Result<Self> {
        if !super::supported() {
            return Err(io::Error::other(
                "Audio CDs are supported on Linux, Windows and macOS",
            ));
        }
        Self::open_driver(device, 11)
    }

    fn open_driver(device: &str, driver: c_int) -> io::Result<Self> {
        let api = Bindings::load()?;
        let device =
            CString::new(device.trim()).map_err(|_| io::Error::other("Invalid CD device name"))?;
        // DRIVER_DEVICE (11) excludes image-file drivers, including for automatic selection.
        // SAFETY: the optional string lives through the call; the result is checked and owned.
        let cdio = NonNull::new(unsafe {
            (api.open)(
                if device.is_empty() {
                    std::ptr::null()
                } else {
                    device.as_ptr()
                },
                driver,
            )
        })
        .ok_or_else(|| {
            io::Error::other(
                "Cannot open the CD drive. Insert an audio CD and check drive access permissions.",
            )
        })?;
        // Establish the change baseline before inspecting the inserted disc.
        // SAFETY: cdio is a valid handle returned above.
        unsafe {
            (api.media_changed)(cdio.as_ptr());
        }
        Ok(Self {
            api,
            cdio,
            cdda: None,
            paranoia: None,
            next_sector: None,
            checked: std::time::Instant::now(),
        })
    }

    pub(super) fn disc(&mut self) -> io::Result<Disc> {
        // SAFETY: cdio is a live handle owned exclusively by this Drive.
        unsafe {
            let first = (self.api.first_track)(self.cdio.as_ptr());
            let last = (self.api.last_track)(self.cdio.as_ptr());
            if first == 0 || last > 99 || first > last {
                return Err(io::Error::other("Cannot read the CD table of contents"));
            }
            let mut tracks = Vec::new();
            for number in first..=last {
                let format = (self.api.track_format)(self.cdio.as_ptr(), number);
                if !(0..=4).contains(&format) {
                    return Err(io::Error::other("Cannot read the CD track format"));
                }
                tracks.push(DiscTrack {
                    number,
                    start: (self.api.track_start)(self.cdio.as_ptr(), number),
                    end: (self.api.track_start)(
                        self.cdio.as_ptr(),
                        if number == last { 0xaa } else { number + 1 },
                    ),
                    audio: format == 0,
                });
            }
            if !tracks.iter().any(|t| t.audio) {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "The inserted disc contains no audio tracks",
                ));
            }
            if tracks.iter().any(|track| !track.audio) {
                // Enhanced CDs need paranoia's first-session lead-out. Ordinary
                // audio TOCs can be checked without its expensive drive calibration.
                let cdda = self.audio_handle()?;
                let end = (self.api.audio_end)(cdda.as_ptr())
                    .checked_add(1)
                    .filter(|end| *end > 0)
                    .ok_or_else(|| io::Error::other("Invalid CD audio lead-out"))?;
                for track in tracks.iter_mut().filter(|track| track.audio) {
                    track.end = track.end.min(end);
                }
            }
            Disc::new(tracks)
        }
    }

    fn audio_handle(&mut self) -> io::Result<NonNull<c_void>> {
        if let Some(cdda) = self.cdda {
            return Ok(cdda);
        }
        // SAFETY: cdio is live; cdda is retained immediately and closed without freeing cdio.
        unsafe {
            let cdda = NonNull::new((self.api.identify)(
                self.cdio.as_ptr(),
                0,
                std::ptr::null_mut(),
            ))
            .ok_or_else(|| io::Error::other("Cannot initialize CD audio reading"))?;
            self.cdda = Some(cdda);
            if (self.api.cdda_open)(cdda.as_ptr()) != 0 {
                return Err(io::Error::other("Cannot read audio from this CD drive"));
            }
            Ok(cdda)
        }
    }

    pub(super) fn prepare(&mut self, track: &DiscTrack) -> io::Result<()> {
        // SAFETY: each successful allocation is immediately retained for Drop; the
        // cdda close variant leaves cdio alive, and paranoia is always freed first.
        unsafe {
            let cdda = self.audio_handle()?;
            if (self.api.channels)(cdda.as_ptr(), track.number) == 4
                || (self.api.preemphasis)(cdda.as_ptr(), track.number) == 1
            {
                return Err(io::Error::other(
                    "Four-channel and pre-emphasized CDs are not supported yet",
                ));
            }
            let paranoia = match self.paranoia {
                Some(paranoia) => paranoia,
                None => NonNull::new((self.api.init)(cdda.as_ptr()))
                    .ok_or_else(|| io::Error::other("Cannot initialize CD error correction"))?,
            };
            self.paranoia = Some(paranoia);
            // Bound retries; a skipped sector is reported as an error by our callback.
            (self.api.mode)(paranoia.as_ptr(), 0xff ^ 0x20);
            (self.api.range)(
                paranoia.as_ptr(),
                track.start.into(),
                (track.end - 1).into(),
            );
            self.next_sector = None;
            (self.api.media_changed)(self.cdio.as_ptr());
            self.checked = std::time::Instant::now();
        }
        Ok(())
    }

    fn read_sector(&mut self, lsn: i32, output: &mut [u8; SECTOR_BYTES]) -> io::Result<()> {
        let paranoia = self
            .paranoia
            .ok_or_else(|| io::Error::other("CD reader is not initialized"))?;
        // SAFETY: all handles are live and exclusively held. The returned buffer contains
        // 1176 native-endian i16 samples and is copied before the next library call.
        unsafe {
            if self.checked.elapsed() >= std::time::Duration::from_secs(1) {
                self.checked = std::time::Instant::now();
                if (self.api.media_changed)(self.cdio.as_ptr()) == 1 {
                    return Err(io::Error::other("The CD was removed or changed"));
                }
            }
            if self.next_sector != Some(lsn) && (self.api.seek)(paranoia.as_ptr(), lsn, 0) < 0 {
                return Err(io::Error::other("Cannot seek to this CD track"));
            }
            SKIPPED.set(false);
            let samples = (self.api.read)(paranoia.as_ptr(), Some(read_status), 10);
            self.next_sector = None;
            if samples.is_null() || SKIPPED.get() {
                return Err(io::Error::other(
                    "Unrecoverable CD read error; clean the disc and try again",
                ));
            }
            for (bytes, sample) in output
                .chunks_exact_mut(2)
                .zip(std::slice::from_raw_parts(samples, SECTOR_BYTES / 2))
            {
                bytes.copy_from_slice(&sample.to_le_bytes());
            }
            self.next_sector = Some(lsn + 1);
        }
        Ok(())
    }
}

/// A persistent monitor uses readiness/change commands, not repeated audio reads.
/// Call from a blocking worker; none of the native operations belong on a UI/runtime thread.
pub struct Monitor {
    api: Arc<Bindings>,
    drives: std::collections::HashMap<String, MonitoredDrive>,
}

struct MonitoredDrive {
    drive: Drive,
    disc: Disc,
    node: Option<super::device::Node>,
    presence: Presence,
}

fn sense_reports_absent(sense: &[u8]) -> bool {
    // Only current "not ready / medium not present" establishes absence.
    // Spin-up (02/04), unit attention (06), timeouts and deferred errors do not.
    match sense.first().map(|byte| byte & 0x7f) {
        Some(0x70) if sense.len() >= 14 && sense[7] >= 6 => {
            sense[2] & 0x0f == 2 && sense[12] == 0x3a
        }
        Some(0x72) if sense.len() >= 4 => sense[1] & 0x0f == 2 && sense[2] == 0x3a,
        _ => false,
    }
}

#[derive(Default)]
struct Presence {
    refresh: bool,
    absent: u8,
    missing: u8,
}

#[derive(Debug, PartialEq, Eq)]
enum MonitorAction {
    Keep,
    Refresh,
    Remove,
}

impl Presence {
    fn observe(&mut self, readiness: Readiness, changed: bool, listed: bool) -> MonitorAction {
        self.refresh |= changed || readiness != Readiness::Ready;
        self.absent = if readiness == Readiness::Absent {
            self.absent.saturating_add(1)
        } else {
            0
        };
        self.missing = if !listed && readiness != Readiness::Ready {
            self.missing.saturating_add(1)
        } else {
            0
        };
        if self.absent >= 2 || self.missing >= 3 {
            MonitorAction::Remove
        } else if readiness == Readiness::Ready && self.refresh {
            MonitorAction::Refresh
        } else {
            MonitorAction::Keep
        }
    }
}

impl Monitor {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            api: Bindings::load()?,
            drives: Default::default(),
        })
    }

    pub fn poll(&mut self) -> Vec<(String, Disc)> {
        let mut devices = Vec::new();
        // SAFETY: libcdio owns a NULL-terminated list; copy strings before freeing it once.
        unsafe {
            let list = (self.api.devices)(11);
            if !list.is_null() {
                let mut at = list;
                while !(*at).is_null() {
                    devices.push(CStr::from_ptr(*at).to_string_lossy().into_owned());
                    at = at.add(1);
                }
                (self.api.free_devices)(list);
            }
        }
        let devices = super::device::normalize(devices);
        let mut candidates = devices.clone();
        candidates.extend(
            self.drives
                .keys()
                .filter(|device| !devices.contains(device))
                .cloned(),
        );
        for device in candidates {
            let node = super::device::Node::read(&device).ok();
            let reconnected = self
                .drives
                .get(&device)
                .is_some_and(|known| node.is_some() && node != known.node);
            #[cfg(target_os = "linux")]
            let readiness = super::device::readiness(&device);
            if let Some(known) = self.drives.get_mut(&device) {
                // SAFETY: only the monitor accesses this live handle.
                let changed = reconnected
                    || unsafe { (self.api.media_changed)(known.drive.cdio.as_ptr()) } == 1;
                #[cfg(not(target_os = "linux"))]
                let readiness = if reconnected {
                    Readiness::Unknown
                } else {
                    known.drive.readiness()
                };
                if readiness == Readiness::Unknown && !known.presence.refresh {
                    tracing::debug!(%device, "CD drive temporarily unavailable; retaining insertion");
                }
                match known
                    .presence
                    .observe(readiness, changed, devices.contains(&device))
                {
                    MonitorAction::Keep if !reconnected || readiness == Readiness::Absent => {
                        continue;
                    }
                    MonitorAction::Remove => {
                        tracing::debug!(%device, ?readiness, "CD removal confirmed");
                        self.drives.remove(&device);
                        continue;
                    }
                    MonitorAction::Keep | MonitorAction::Refresh => {}
                }
            }
            #[cfg(target_os = "linux")]
            if readiness == Readiness::Absent {
                continue;
            }
            // Keep the old identity until a fresh TOC succeeds. A busy drive or a
            // failed probe must not stop playback, purge its queue or re-prompt.
            if let Ok(mut drive) = Drive::open(&device)
                && drive.readiness() != Readiness::Absent
            {
                match drive.disc() {
                    Ok(disc) => {
                        self.drives.insert(
                            device,
                            MonitoredDrive {
                                drive,
                                disc,
                                node,
                                presence: Presence::default(),
                            },
                        );
                    }
                    Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                        // A successfully read data-only TOC confirms replacement.
                        self.drives.remove(&device);
                    }
                    Err(error) => tracing::debug!(%device, %error, "CD identification deferred"),
                }
            }
        }
        self.drives
            .iter()
            .map(|(device, known)| (device.clone(), known.disc.clone()))
            .collect()
    }
}

thread_local! { static SKIPPED: Cell<bool> = const { Cell::new(false) }; }

unsafe extern "C" fn read_status(_: c_long, status: c_int) {
    if status == 6 {
        SKIPPED.set(true);
    }
}

impl Drop for Drive {
    fn drop(&mut self) {
        // SAFETY: each handle is uniquely owned and destroyed in dependency order.
        unsafe {
            if let Some(p) = self.paranoia.take() {
                (self.api.free)(p.as_ptr());
            }
            if let Some(d) = self.cdda.take() {
                (self.api.cdda_close)(d.as_ptr());
            }
            (self.api.destroy)(self.cdio.as_ptr());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Seek, SeekFrom};

    #[test]
    fn audio_cd_queued_track_requests_cannot_supersede_a_newer_selection() {
        let reader = Reader::new("unused-device");
        let old = reader.request();
        let newest = reader.request();
        assert!(
            old.open("unused-track")
                .err()
                .unwrap()
                .to_string()
                .contains("superseded")
        );
        newest.check_current().unwrap();
        let mut old = newest;
        let _newest = reader.request();
        assert!(
            old.sector(0, &mut [0; SECTOR_BYTES])
                .unwrap_err()
                .to_string()
                .contains("superseded")
        );
    }

    #[test]
    fn audio_cd_monitor_preserves_insertion_through_busy_and_failed_probes() {
        let mut presence = Presence::default();
        assert_eq!(
            presence.observe(Readiness::Ready, false, true),
            MonitorAction::Keep
        );
        // A busy drive can remain unready for many polls without being removed.
        for _ in 0..20 {
            assert_eq!(
                presence.observe(Readiness::Unknown, false, true),
                MonitorAction::Keep
            );
        }
        assert_eq!(
            presence.observe(Readiness::Ready, false, true),
            MonitorAction::Refresh
        );
        // An unsuccessful TOC refresh retains the disc and retries once ready.
        assert_eq!(
            presence.observe(Readiness::Unknown, true, true),
            MonitorAction::Keep
        );
        assert_eq!(
            presence.observe(Readiness::Ready, false, true),
            MonitorAction::Refresh
        );
    }

    #[test]
    fn audio_cd_monitor_requires_confirmed_absence_and_handles_unplugging() {
        let mut presence = Presence::default();
        assert_eq!(
            presence.observe(Readiness::Absent, true, true),
            MonitorAction::Keep
        );
        assert_eq!(
            presence.observe(Readiness::Ready, false, true),
            MonitorAction::Refresh
        );
        assert_eq!(
            presence.observe(Readiness::Absent, true, true),
            MonitorAction::Keep
        );
        assert_eq!(
            presence.observe(Readiness::Absent, false, true),
            MonitorAction::Remove
        );
        let mut presence = Presence::default();
        assert_eq!(
            presence.observe(Readiness::Unknown, false, false),
            MonitorAction::Keep
        );
        // A temporary enumeration failure is also inconclusive if the drive responds.
        assert_eq!(
            presence.observe(Readiness::Ready, false, false),
            MonitorAction::Refresh
        );
        assert_eq!(
            presence.observe(Readiness::Unknown, false, false),
            MonitorAction::Keep
        );
        assert_eq!(
            presence.observe(Readiness::Unknown, false, false),
            MonitorAction::Keep
        );
        assert_eq!(
            presence.observe(Readiness::Unknown, false, false),
            MonitorAction::Remove
        );
        let mut presence = Presence::default();
        assert_eq!(
            presence.observe(Readiness::Ready, true, true),
            MonitorAction::Refresh
        );
    }

    #[test]
    fn audio_cd_monitor_distinguishes_absence_from_spinup_attention_and_bad_sense() {
        let mut sense = [0; 18];
        sense[0] = 0x70;
        sense[2] = 2;
        sense[7] = 10;
        sense[12] = 0x3a;
        assert!(sense_reports_absent(&sense));
        sense[12] = 4;
        sense[13] = 1; // Becoming ready.
        assert!(!sense_reports_absent(&sense));
        sense[2] = 6;
        sense[12] = 0x28; // Unit attention after a media-change/reset event.
        assert!(!sense_reports_absent(&sense));
        assert!(sense_reports_absent(&[0x72, 2, 0x3a, 2]));
        for invalid in [
            &[][..],
            &[0x70, 2, 0x3a, 0],
            &[0x73, 2, 0x3a, 0],
            &[0x72, 6, 0x3a, 0],
        ] {
            assert!(!sense_reports_absent(invalid));
        }
    }

    #[test]
    #[ignore = "requires the optional libcdio and libcdio-paranoia runtime libraries"]
    fn audio_cd_native_fixture_reads_and_seeks_exact_pcm() {
        let root = tempfile::tempdir().unwrap();
        let mut pcm = Vec::new();
        for sample in 0..(300 * 588) {
            // Distinct tracks expose stale audio when reusing the native reader.
            let amplitude = if sample < 150 * 588 { 16000.0 } else { 8000.0 };
            let value = ((sample as f64 * 440.0 * std::f64::consts::TAU / 44100.0).sin()
                * amplitude) as i16;
            pcm.extend_from_slice(&value.to_le_bytes());
            pcm.extend_from_slice(&value.to_le_bytes());
        }
        std::fs::write(root.path().join("disc.bin"), &pcm).unwrap();
        let cue = root.path().join("disc.cue");
        std::fs::write(&cue, "FILE \"disc.bin\" BINARY\n  TRACK 01 AUDIO\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 01 00:02:00\n").unwrap();
        // Image access is a test seam only; production always selects DRIVER_DEVICE.
        let mut drive = Drive::open_driver(cue.to_str().unwrap(), 9).unwrap();
        let disc = drive.disc().unwrap();
        assert_eq!(disc.tracks.len(), 2);
        assert_eq!(disc.tracks[0].start, 0);
        assert_eq!(disc.tracks[0].end, 150);
        assert!(
            drive.cdda.is_none(),
            "TOC inspection should not calibrate the drive"
        );
        drop(drive);
        let mut reader = Reader::new(cue.to_str().unwrap());
        reader.driver = 9;
        let mut stream = reader.request().open(&disc.key(&disc.tracks[0])).unwrap();
        stream
            .seek(SeekFrom::Start(44 + 75 * SECTOR_BYTES as u64))
            .unwrap();
        let mut sector = [0; SECTOR_BYTES];
        stream.read_exact(&mut sector).unwrap();
        assert_eq!(&sector, &pcm[75 * SECTOR_BYTES..76 * SECTOR_BYTES]);
        stream.seek(SeekFrom::Start(44)).unwrap();
        let mut decoded = Vec::new();
        stream.read_to_end(&mut decoded).unwrap();
        assert_eq!(decoded, pcm[..150 * SECTOR_BYTES]);

        let (cdda, paranoia) = {
            let state = reader.shared.drive.lock().unwrap();
            let drive = &state.as_ref().unwrap().drive;
            (drive.cdda.unwrap(), drive.paranoia.unwrap())
        };
        let mut old_reader = reader.request();
        for index in [1, 0, 1] {
            let mut stream = reader
                .request()
                .open(&disc.key(&disc.tracks[index]))
                .unwrap();
            stream.seek(SeekFrom::Start(44)).unwrap();
            decoded.clear();
            stream.read_to_end(&mut decoded).unwrap();
            let start = disc.tracks[index].start as usize * SECTOR_BYTES;
            let end = disc.tracks[index].end as usize * SECTOR_BYTES;
            assert_eq!(decoded, pcm[start..end]);
            let state = reader.shared.drive.lock().unwrap();
            let drive = &state.as_ref().unwrap().drive;
            assert_eq!(drive.cdda, Some(cdda));
            assert_eq!(drive.paranoia, Some(paranoia));
        }
        assert!(
            old_reader
                .sector(0, &mut sector)
                .unwrap_err()
                .to_string()
                .contains("superseded")
        );
        assert!(reader.request().open("different-disc-01").is_err());
    }
}

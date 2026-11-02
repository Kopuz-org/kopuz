//! Keep drive latency off the decoder thread. Memory is bounded to 30 seconds
//! of PCM. Starts/seeks need only a short reserve; an underrun refills more.

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Condvar, Mutex};

use super::{DiscTrack, SECTOR_BYTES, SectorReader};

const CAPACITY: usize = 30 * 75;
const RESERVE: usize = 5 * 75;
const STARTUP: usize = 20;

struct State {
    start: i32,
    wanted: i32,
    sectors: VecDeque<[u8; SECTOR_BYTES]>,
    error: Option<String>,
    stopped: bool,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

pub(super) struct ReadAhead {
    shared: Arc<Shared>,
    end: i32,
    reserve: usize,
    startup: usize,
}

impl ReadAhead {
    pub(super) fn new(reader: impl SectorReader + 'static, track: &DiscTrack) -> io::Result<Self> {
        Self::with_limits(reader, track, CAPACITY, RESERVE)
    }

    fn with_limits(
        mut reader: impl SectorReader + 'static,
        track: &DiscTrack,
        capacity: usize,
        reserve: usize,
    ) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                start: track.start,
                wanted: track.start - 2,
                sectors: VecDeque::new(),
                error: None,
                stopped: false,
            }),
            ready: Condvar::new(),
        });
        let worker = shared.clone();
        let end = track.end;
        std::thread::Builder::new()
            .name("cd-read-ahead".into())
            .spawn(move || {
                loop {
                    let Ok(mut state) = worker.state.lock() else {
                        return;
                    };
                    while !state.stopped
                        && (state.error.is_some()
                            || state.sectors.len() >= capacity
                            || state.start + state.sectors.len() as i32 >= end)
                    {
                        state = match worker.ready.wait(state) {
                            Ok(state) => state,
                            Err(_) => return,
                        };
                    }
                    if state.stopped {
                        return;
                    }
                    let start = state.start;
                    let lsn = start + state.sectors.len() as i32;
                    drop(state);
                    let mut sector = [0; SECTOR_BYTES];
                    let result = reader.sector(lsn, &mut sector);
                    let Ok(mut state) = worker.state.lock() else {
                        return;
                    };
                    if state.stopped {
                        return;
                    }
                    // A seek may supersede an in-flight drive read.
                    if state.start + state.sectors.len() as i32 == lsn {
                        match result {
                            Ok(()) => state.sectors.push_back(sector),
                            Err(error) => state.error = Some(error.to_string()),
                        }
                    }
                    worker.ready.notify_all();
                }
            })?;
        Ok(Self {
            shared,
            end,
            reserve,
            startup: reserve.min(STARTUP),
        })
    }
}

impl SectorReader for ReadAhead {
    fn sector(&mut self, lsn: i32, output: &mut [u8; SECTOR_BYTES]) -> io::Result<()> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| io::Error::other("CD buffer lock poisoned"))?;
        let contiguous = lsn == state.wanted || lsn == state.wanted + 1;
        let buffered_end = state.start + state.sectors.len() as i32;
        if lsn < state.start || lsn > buffered_end {
            state.start = lsn;
            state.sectors.clear();
            state.error = None;
        } else {
            let consumed = (lsn - state.start) as usize;
            state.sectors.drain(..consumed);
            state.start = lsn;
        }
        let empty = state.sectors.is_empty();
        state.wanted = lsn;
        self.shared.ready.notify_all();
        let needed = if !contiguous {
            self.startup.min((self.end - lsn) as usize)
        } else if empty {
            self.reserve.min((self.end - lsn) as usize)
        } else {
            1
        };
        loop {
            if state.sectors.len() >= needed || (state.error.is_some() && !state.sectors.is_empty())
            {
                output.copy_from_slice(&state.sectors[0]);
                return Ok(());
            }
            if let Some(error) = &state.error {
                return Err(io::Error::other(error.clone()));
            }
            state = self
                .shared
                .ready
                .wait(state)
                .map_err(|_| io::Error::other("CD buffer lock poisoned"))?;
        }
    }
}

impl Drop for ReadAhead {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.stopped = true;
            self.shared.ready.notify_all();
        }
        // Never join a potentially slow hardware read on the player/UI thread.
        // The worker drops the drive as soon as that read completes.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    struct Reader {
        reads: mpsc::Sender<i32>,
        stall: Option<(i32, mpsc::Receiver<()>)>,
    }
    impl SectorReader for Mutex<Reader> {
        fn sector(&mut self, lsn: i32, output: &mut [u8; SECTOR_BYTES]) -> io::Result<()> {
            let reader = self.get_mut().unwrap();
            let _ = reader.reads.send(lsn);
            if let Some((at, release)) = &reader.stall
                && lsn == *at
            {
                release.recv_timeout(Duration::from_secs(5)).unwrap();
            }
            output.fill(lsn as u8);
            Ok(())
        }
    }
    fn track() -> DiscTrack {
        DiscTrack {
            number: 1,
            start: 0,
            end: 100,
            audio: true,
        }
    }

    #[test]
    fn audio_cd_starts_and_seeks_without_waiting_for_the_full_reserve() {
        for start in [0, 50] {
            let (sent, reads) = mpsc::channel();
            let (release, blocked) = mpsc::channel();
            let stalled_at = start + STARTUP as i32;
            let reader = Mutex::new(Reader {
                reads: sent,
                stall: Some((stalled_at, blocked)),
            });
            let mut buffer = ReadAhead::with_limits(reader, &track(), 100, 75).unwrap();
            while reads.recv_timeout(Duration::from_secs(2)).unwrap() != stalled_at {}
            let (sent, finished) = mpsc::channel();
            let consumer = std::thread::spawn(move || {
                let mut output = [0; SECTOR_BYTES];
                let result = buffer.sector(start, &mut output);
                sent.send((result, output)).unwrap();
            });
            // The short reserve is available, but filling the full playback
            // reserve would require the blocked hardware read to complete.
            let result = finished.recv_timeout(Duration::from_secs(2));
            release.send(()).unwrap();
            consumer.join().unwrap();
            let (result, output) = result.expect("start/seek waited for the full reserve");
            result.unwrap();
            assert!(output.iter().all(|byte| *byte == start as u8));
        }
    }

    #[test]
    fn audio_cd_buffer_covers_a_stalled_drive_and_bounds_reads() {
        let (sent, reads) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let reader = Mutex::new(Reader {
            reads: sent,
            stall: Some((8, blocked)),
        });
        let mut buffer = ReadAhead::with_limits(reader, &track(), 8, 4).unwrap();
        let mut output = [0; SECTOR_BYTES];
        buffer.sector(0, &mut output).unwrap();
        for _ in 0..8 {
            reads.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        assert!(reads.recv_timeout(Duration::from_millis(30)).is_err());
        buffer.sector(1, &mut output).unwrap();
        assert_eq!(reads.recv_timeout(Duration::from_secs(2)).unwrap(), 8);
        // The drive is blocked until we release it. These reads must use the reserve.
        for lsn in 2..8 {
            buffer.sector(lsn, &mut output).unwrap();
            assert_eq!(output[0], lsn as u8);
        }
        release.send(()).unwrap();
        buffer.sector(8, &mut output).unwrap();
        assert_eq!(output[0], 8);
    }

    #[test]
    fn audio_cd_buffer_seeks_discard_stale_audio_and_wake_a_full_worker() {
        let (sent, _reads) = mpsc::channel();
        let reader = Mutex::new(Reader {
            reads: sent,
            stall: None,
        });
        let mut buffer = ReadAhead::with_limits(reader, &track(), 8, 4).unwrap();
        let mut output = [0; SECTOR_BYTES];
        for lsn in [0, 1, 50, 51, 3, 99] {
            buffer.sector(lsn, &mut output).unwrap();
            assert!(output.iter().all(|byte| *byte == lsn as u8));
        }
    }
    #[test]
    fn audio_cd_buffer_propagates_read_errors_and_releases_its_worker() {
        struct Failing(mpsc::Sender<()>);
        impl SectorReader for Failing {
            fn sector(&mut self, lsn: i32, output: &mut [u8; SECTOR_BYTES]) -> io::Result<()> {
                if lsn >= 2 {
                    return Err(io::Error::other("disc removed"));
                }
                output.fill(lsn as u8);
                Ok(())
            }
        }
        impl Drop for Failing {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let (sent, ended) = mpsc::channel();
        let mut buffer = ReadAhead::with_limits(Failing(sent), &track(), 8, 4).unwrap();
        let mut output = [0; SECTOR_BYTES];
        buffer.sector(0, &mut output).unwrap();
        buffer.sector(1, &mut output).unwrap();
        assert_eq!(output[0], 1);
        assert!(
            buffer
                .sector(2, &mut output)
                .unwrap_err()
                .to_string()
                .contains("disc removed")
        );
        drop(buffer);
        ended.recv_timeout(Duration::from_secs(2)).unwrap();
    }
}

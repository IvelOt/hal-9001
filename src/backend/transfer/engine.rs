use std::fs::File;
use std::io::{self, BufReader, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;

use super::align::AlignedBuf;
use super::rate::RateMeter;
use super::sink::{TransferSink, DEFAULT_CHUNK_SIZE};
use super::sys::{fadvise_dontneed, fadvise_sequential};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferPhase {
    Streaming,
    Flushing,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransferProgress {
    pub written_bytes: u64,
    pub persisted_bytes: u64,
    pub total_bytes: u64,
    pub speed_mbps: f64,
    pub avg_mbps: f64,
    pub eta_secs: u64,
    pub phase: TransferPhase,
}

pub enum TransferSource {
    Plain {
        file: File,
        fd: RawFd,
        offset: u64,
        total_bytes: u64,
    },
    Gzip {
        decoder: Box<BufReader<GzDecoder<File>>>,
        raw_fd: RawFd,
        total_bytes: u64,
    },
}

impl TransferSource {
    pub fn open_plain(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let total_bytes = file.metadata()?.len();
        let fd = file.as_raw_fd();
        fadvise_sequential(fd);
        Ok(Self::Plain {
            file,
            fd,
            offset: 0,
            total_bytes,
        })
    }

    pub fn open_gzip(path: &Path, uncompressed_hint: u64) -> io::Result<Self> {
        let file = File::open(path)?;
        let raw_fd = file.as_raw_fd();
        fadvise_sequential(raw_fd);
        let decoder = Box::new(BufReader::with_capacity(
            DEFAULT_CHUNK_SIZE,
            GzDecoder::new(file),
        ));
        Ok(Self::Gzip {
            decoder,
            raw_fd,
            total_bytes: uncompressed_hint,
        })
    }

    pub fn total_bytes(&self) -> u64 {
        match self {
            Self::Plain { total_bytes, .. } | Self::Gzip { total_bytes, .. } => *total_bytes,
        }
    }

    pub fn read_full_chunk(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut filled = 0usize;
        match self {
            Self::Plain {
                file, fd, offset, ..
            } => {
                let start_off = *offset;
                while filled < buf.len() {
                    match file.read(&mut buf[filled..]) {
                        Ok(0) => break,
                        Ok(k) => filled += k,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e),
                    }
                }
                if filled > 0 {
                    *offset += filled as u64;
                    // Drop the source pages from the host page cache now that we've read them.
                    fadvise_dontneed(*fd, start_off, filled as u64);
                }
                Ok(filled)
            }
            Self::Gzip {
                decoder, raw_fd, ..
            } => {
                while filled < buf.len() {
                    match decoder.read(&mut buf[filled..]) {
                        Ok(0) => {
                            fadvise_dontneed(*raw_fd, 0, 0);
                            break;
                        }
                        Ok(k) => filled += k,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e),
                    }
                }
                Ok(filled)
            }
        }
    }
}

pub struct TransferConfig {
    pub chunk_size: usize,
    pub progress_throttle: Duration,
    pub cancel_message: String,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self {
            chunk_size: DEFAULT_CHUNK_SIZE,
            progress_throttle: Duration::from_millis(80),
            cancel_message: "transfer cancelled".to_string(),
        }
    }
}

pub struct TransferEngine;

impl TransferEngine {
    pub fn run<F>(
        mut source: TransferSource,
        mut sink: Box<dyn TransferSink>,
        cancel: &AtomicBool,
        cfg: &TransferConfig,
        mut on_progress: F,
    ) -> anyhow::Result<u64>
    where
        F: FnMut(TransferProgress),
    {
        let total_bytes = source.total_bytes();
        let mut buf = AlignedBuf::new_4k(cfg.chunk_size.max(4096));
        let mut written_bytes: u64 = 0;
        let mut persisted_bytes: u64 = 0;
        let now = Instant::now();
        let mut meter = RateMeter::new(now);
        let mut last_emit = now;

        loop {
            if cancel.load(Ordering::Relaxed) {
                sink.abort();
                anyhow::bail!("{}", cfg.cancel_message);
            }

            let n = match source.read_full_chunk(buf.as_mut_slice()) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    sink.abort();
                    return Err(e.into());
                }
            };

            if cancel.load(Ordering::Relaxed) {
                sink.abort();
                anyhow::bail!("{}", cfg.cancel_message);
            }

            match sink.write_chunk(&buf.as_slice()[..n]) {
                Ok(p) => {
                    written_bytes += n as u64;
                    persisted_bytes = p;
                }
                Err(e) => {
                    sink.abort();
                    return Err(e.into());
                }
            }

            let tick = Instant::now();
            if tick.saturating_duration_since(last_emit) >= cfg.progress_throttle {
                // While the first window chunk is in flight, report the conservative progress
                // so the bar moves smoothly without ever outrunning the window by more than 1 chunk.
                let effective_persisted = if persisted_bytes > 0 {
                    persisted_bytes
                } else {
                    written_bytes.min(cfg.chunk_size as u64)
                };
                let sample =
                    meter.record(tick, effective_persisted, total_bytes.max(written_bytes));
                on_progress(TransferProgress {
                    written_bytes,
                    persisted_bytes: effective_persisted,
                    total_bytes: total_bytes.max(written_bytes),
                    speed_mbps: sample.speed_mbps,
                    avg_mbps: sample.avg_mbps,
                    eta_secs: sample.eta_secs,
                    phase: TransferPhase::Streaming,
                });
                last_emit = tick;
            }
        }

        if cancel.load(Ordering::Relaxed) {
            sink.abort();
            anyhow::bail!("{}", cfg.cancel_message);
        }

        let flush_tick = Instant::now();
        let flush_sample = meter.record(
            flush_tick,
            persisted_bytes.max(written_bytes.saturating_sub(cfg.chunk_size as u64 * 2)),
            total_bytes.max(written_bytes),
        );
        on_progress(TransferProgress {
            written_bytes,
            persisted_bytes,
            total_bytes: total_bytes.max(written_bytes),
            speed_mbps: flush_sample.speed_mbps,
            avg_mbps: flush_sample.avg_mbps,
            eta_secs: flush_sample.eta_secs,
            phase: TransferPhase::Flushing,
        });

        let final_persisted = sink.finalize()?;
        let done_tick = Instant::now();
        let done_sample = meter.record(done_tick, final_persisted, final_persisted);
        on_progress(TransferProgress {
            written_bytes: final_persisted,
            persisted_bytes: final_persisted,
            total_bytes: final_persisted,
            speed_mbps: done_sample.avg_mbps,
            avg_mbps: done_sample.avg_mbps,
            eta_secs: 0,
            phase: TransferPhase::Done,
        });

        Ok(final_persisted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::transfer::sink::{
        is_temp_part_file, temp_part_path, BlockDirectSink, WindowedSink,
    };

    #[test]
    fn windowed_sink_writes_atomically_via_part_file_and_renames_on_finalize() {
        let dir = tempfile::tempdir().unwrap();
        let src_path = dir.path().join("source.iso");
        let dst_path = dir.path().join("ubuntu.iso");
        let part_path = temp_part_path(&dst_path);

        // 3 chunks of 64 KiB + 1234 unaligned tail bytes
        let payload: Vec<u8> = (0..(64 * 1024 * 3 + 1234))
            .map(|i| ((i * 31 + 7) & 0xFF) as u8)
            .collect();
        std::fs::write(&src_path, &payload).unwrap();

        let source = TransferSource::open_plain(&src_path).unwrap();
        let sink = Box::new(WindowedSink::with_window(&dst_path, 2).unwrap());

        // Before finalize, only the .hal9001-part file exists, not the final .iso
        assert!(part_path.exists());
        assert!(!dst_path.exists());
        assert!(is_temp_part_file(
            part_path.file_name().unwrap().to_str().unwrap()
        ));

        let cancel = AtomicBool::new(false);
        let cfg = TransferConfig {
            chunk_size: 64 * 1024,
            progress_throttle: Duration::ZERO,
            cancel_message: "cancelled".to_string(),
        };

        let mut events = Vec::new();
        let total = TransferEngine::run(source, sink, &cancel, &cfg, |p| events.push(p)).unwrap();

        assert_eq!(total, payload.len() as u64);
        assert!(dst_path.exists(), "final ISO must exist after finalize");
        assert!(
            !part_path.exists(),
            "temporary .hal9001-part file must be gone after atomic rename"
        );
        assert_eq!(std::fs::read(&dst_path).unwrap(), payload);
        assert!(events.iter().any(|e| e.phase == TransferPhase::Flushing));
        assert_eq!(events.last().unwrap().phase, TransferPhase::Done);
        assert_eq!(events.last().unwrap().persisted_bytes, payload.len() as u64);
    }

    #[test]
    fn windowed_sink_removes_part_file_on_cancel_without_leaving_zombie_iso() {
        let dir = tempfile::tempdir().unwrap();
        let src_path = dir.path().join("source.iso");
        let dst_path = dir.path().join("aborted.iso");
        let part_path = temp_part_path(&dst_path);

        let payload = vec![0x42u8; 256 * 1024];
        std::fs::write(&src_path, &payload).unwrap();

        let source = TransferSource::open_plain(&src_path).unwrap();
        let sink = Box::new(WindowedSink::with_window(&dst_path, 2).unwrap());
        let cancel = AtomicBool::new(false);
        let cfg = TransferConfig {
            chunk_size: 64 * 1024,
            progress_throttle: Duration::ZERO,
            cancel_message: "cancelled by user".to_string(),
        };

        let res = TransferEngine::run(source, sink, &cancel, &cfg, |_p| {
            // Trigger cancellation after the first progress callback
            cancel.store(true, Ordering::Relaxed);
        });

        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("cancelled by user"));
        assert!(
            !dst_path.exists(),
            "aborted transfer must never create final .iso"
        );
        assert!(
            !part_path.exists(),
            "aborted transfer must remove temporary .hal9001-part file"
        );
    }

    #[test]
    fn block_direct_sink_handles_4k_aligned_chunks_and_unaligned_tail() {
        let dir = tempfile::tempdir().unwrap();
        let src_path = dir.path().join("raw.img");
        let dst_path = dir.path().join("device.bin");

        // 4096 * 4 + 537 bytes (unaligned tail)
        let payload: Vec<u8> = (0..(4096 * 4 + 537))
            .map(|i| ((i * 17 + 3) & 0xFF) as u8)
            .collect();
        std::fs::write(&src_path, &payload).unwrap();
        std::fs::write(&dst_path, b"").unwrap();

        let source = TransferSource::open_plain(&src_path).unwrap();
        let sink = Box::new(BlockDirectSink::open(&dst_path).unwrap());
        let cancel = AtomicBool::new(false);
        let cfg = TransferConfig {
            chunk_size: 8192,
            progress_throttle: Duration::ZERO,
            cancel_message: "cancelled".to_string(),
        };

        let written = TransferEngine::run(source, sink, &cancel, &cfg, |_| {}).unwrap();
        assert_eq!(written, payload.len() as u64);
        assert_eq!(std::fs::read(&dst_path).unwrap(), payload);
    }
}

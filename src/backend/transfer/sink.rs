use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use super::align::{AlignedBuf, DIRECT_ALIGN};
use super::sys::{
    blkflsbuf_best_effort, clear_o_direct, fadvise_dontneed, fsync_dir, sync_range_async,
    sync_range_wait,
};

pub const PART_SUFFIX: &str = ".hal9001-part";
pub const DEFAULT_CHUNK_SIZE: usize = 4 * 1024 * 1024; // 4 MiB
pub const DEFAULT_WINDOW_CHUNKS: usize = 2; // 2 * 4 MiB = 8 MiB max dirty pages in flight

/// Build the hidden temporary path `.{filename}.hal9001-part` in the same directory
/// as `final_path` so `rename` on completion is atomic on the same filesystem.
pub fn temp_part_path(final_path: &Path) -> PathBuf {
    let parent = final_path.parent().unwrap_or_else(|| Path::new("."));
    let name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "image.iso".to_string());
    parent.join(format!(".{name}{PART_SUFFIX}"))
}

/// Returns true if a directory entry filename is a temporary in-progress or orphaned
/// `.hal9001-part` transfer file.
pub fn is_temp_part_file(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(PART_SUFFIX)
}

pub trait TransferSink: Send {
    /// Write a chunk of bytes and return the cumulative number of bytes that have
    /// been physically persisted to the underlying storage medium so far.
    fn write_chunk(&mut self, buf: &[u8]) -> io::Result<u64>;

    /// Flush and wait for all remaining in-flight data, atomically publish any
    /// temporary file, sync directory metadata, and return total persisted bytes.
    fn finalize(self: Box<Self>) -> io::Result<u64>;

    /// Abort the transfer and clean up any temporary state or partial files.
    fn abort(self: Box<Self>);
}

/// Controlled-streaming sink for files on mounted filesystems (FAT32, exFAT, NTFS, ext4).
///
/// Writes into a hidden `.{name}.hal9001-part` temporary file, initiates asynchronous
/// writeout (`SYNC_FILE_RANGE_WRITE`) on each chunk, and waits (`WAIT_BEFORE | WRITE | WAIT_AFTER`)
/// and evicts clean page cache (`POSIX_FADV_DONTNEED`) on chunk `N - max_in_flight_chunks`.
/// Dirty memory in the kernel page cache is strictly bounded to `max_in_flight_chunks * chunk_size`
/// (default 8 MiB), preventing system freezes and dishonest 99% progress bars.
pub struct WindowedSink {
    file: Option<File>,
    fd: RawFd,
    temp_path: PathBuf,
    final_path: PathBuf,
    written_bytes: u64,
    persisted_bytes: u64,
    in_flight: VecDeque<(u64, u64)>,
    max_in_flight_chunks: usize,
    use_fdatasync_fallback: bool,
    chunks_since_fdatasync: usize,
    completed: bool,
}

impl WindowedSink {
    pub fn create(final_path: &Path) -> io::Result<Self> {
        Self::with_window(final_path, DEFAULT_WINDOW_CHUNKS)
    }

    pub fn with_window(final_path: &Path, max_in_flight_chunks: usize) -> io::Result<Self> {
        let temp_path = temp_part_path(final_path);
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC)
            .open(&temp_path)?;
        let fd = file.as_raw_fd();
        Ok(Self {
            file: Some(file),
            fd,
            temp_path,
            final_path: final_path.to_path_buf(),
            written_bytes: 0,
            persisted_bytes: 0,
            in_flight: VecDeque::with_capacity(max_in_flight_chunks.max(1) + 2),
            max_in_flight_chunks: max_in_flight_chunks.max(1),
            use_fdatasync_fallback: false,
            chunks_since_fdatasync: 0,
            completed: false,
        })
    }

    pub fn temp_path(&self) -> &Path {
        &self.temp_path
    }
}

impl TransferSink for WindowedSink {
    fn write_chunk(&mut self, buf: &[u8]) -> io::Result<u64> {
        if buf.is_empty() {
            return Ok(self.persisted_bytes);
        }
        let file = self.file.as_mut().expect("sink file open");
        file.write_all(buf)?;

        let chunk_off = self.written_bytes;
        let chunk_len = buf.len() as u64;
        self.written_bytes += chunk_len;

        if !self.use_fdatasync_fallback {
            match sync_range_async(self.fd, chunk_off, chunk_len) {
                Ok(()) => {
                    self.in_flight.push_back((chunk_off, chunk_len));
                    while self.in_flight.len() >= self.max_in_flight_chunks {
                        let (old_off, old_len) = self.in_flight.pop_front().unwrap();
                        if sync_range_wait(self.fd, old_off, old_len).is_err() {
                            self.use_fdatasync_fallback = true;
                            file.sync_data()?;
                            fadvise_dontneed(self.fd, 0, self.written_bytes);
                            self.in_flight.clear();
                            self.persisted_bytes = self.written_bytes;
                            return Ok(self.persisted_bytes);
                        }
                        fadvise_dontneed(self.fd, old_off, old_len);
                        self.persisted_bytes += old_len;
                    }
                    return Ok(self.persisted_bytes);
                }
                Err(_) => {
                    self.use_fdatasync_fallback = true;
                    self.in_flight.clear();
                }
            }
        }

        // Fallback when sync_file_range is unsupported by the kernel/filesystem:
        // periodic fdatasync every `max_in_flight_chunks` chunks + page cache eviction.
        self.chunks_since_fdatasync += 1;
        if self.chunks_since_fdatasync >= self.max_in_flight_chunks {
            file.sync_data()?;
            fadvise_dontneed(self.fd, 0, self.written_bytes);
            self.persisted_bytes = self.written_bytes;
            self.chunks_since_fdatasync = 0;
        }
        Ok(self.persisted_bytes)
    }

    fn finalize(mut self: Box<Self>) -> io::Result<u64> {
        if let Some(file) = self.file.take() {
            if !self.use_fdatasync_fallback {
                while let Some((off, len)) = self.in_flight.pop_front() {
                    let _ = sync_range_wait(self.fd, off, len);
                    fadvise_dontneed(self.fd, off, len);
                    self.persisted_bytes += len;
                }
            }
            file.sync_all()?;
            fadvise_dontneed(self.fd, 0, self.written_bytes);
            self.persisted_bytes = self.written_bytes;
            drop(file);
        }

        std::fs::rename(&self.temp_path, &self.final_path)?;
        if let Some(parent) = self.final_path.parent() {
            let _ = fsync_dir(parent);
        }
        self.completed = true;
        Ok(self.persisted_bytes)
    }

    fn abort(mut self: Box<Self>) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.temp_path);
        self.completed = true;
    }
}

impl Drop for WindowedSink {
    fn drop(&mut self) {
        if !self.completed {
            drop(self.file.take());
            let _ = std::fs::remove_file(&self.temp_path);
        }
    }
}

/// Unbuffered `O_DIRECT` sink for raw block devices (`/dev/sdX`, `/dev/nvmeXnY`, `/dev/mmcblkX`).
///
/// Opens the target with `O_WRONLY | O_DIRECT | O_EXCL | O_CLOEXEC` and writes using a
/// 4096-byte aligned buffer (`AlignedBuf`), bypassing the kernel page cache entirely.
/// If an image has an unaligned final tail (< 4096 bytes), `O_DIRECT` is cleared via
/// `fcntl(F_SETFL)` for the tail bytes and immediately synced with `fdatasync`.
/// If the target path does not support `O_DIRECT` (e.g. a regular file on `tmpfs` during
/// unit tests), it transparently falls back to synchronous per-chunk `fdatasync` + `fadvise`.
pub struct BlockDirectSink {
    file: File,
    fd: RawFd,
    direct_enabled: bool,
    aligned_buf: Option<AlignedBuf>,
    persisted_bytes: u64,
}

impl BlockDirectSink {
    pub fn open(dev_path: &Path) -> io::Result<Self> {
        // First try O_DIRECT | O_EXCL (standard for raw block devices).
        let direct_try = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_DIRECT | libc::O_EXCL | libc::O_CLOEXEC)
            .open(dev_path);

        let (file, direct_enabled) = match direct_try {
            Ok(f) => (f, true),
            Err(e) if e.raw_os_error() == Some(libc::EINVAL) => {
                // Regular file on tmpfs or filesystem that rejects O_DIRECT/O_EXCL:
                // try O_DIRECT without O_EXCL first, then plain open with per-chunk sync.
                match OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_DIRECT | libc::O_CLOEXEC)
                    .open(dev_path)
                {
                    Ok(f) => (f, true),
                    Err(_) => {
                        let f = OpenOptions::new()
                            .write(true)
                            .custom_flags(libc::O_CLOEXEC)
                            .open(dev_path)?;
                        (f, false)
                    }
                }
            }
            Err(e) => return Err(e),
        };

        let fd = file.as_raw_fd();
        Ok(Self {
            file,
            fd,
            direct_enabled,
            aligned_buf: None,
            persisted_bytes: 0,
        })
    }

    pub fn is_direct(&self) -> bool {
        self.direct_enabled
    }

    fn write_aligned_direct(&mut self, data: &[u8]) -> io::Result<()> {
        debug_assert_eq!(data.len() % DIRECT_ALIGN, 0);
        let mut offset = 0usize;
        while offset < data.len() {
            let src = &data[offset..];
            let result = if src.as_ptr() as usize % DIRECT_ALIGN == 0 {
                self.file.write_all(src).map(|()| src.len())
            } else {
                let bounce = self
                    .aligned_buf
                    .get_or_insert_with(|| AlignedBuf::new_4k(DEFAULT_CHUNK_SIZE));
                let step = src.len().min(bounce.len());
                debug_assert_eq!(step % DIRECT_ALIGN, 0);
                let dst_slice = &mut bounce.as_mut_slice()[..step];
                dst_slice.copy_from_slice(&src[..step]);
                self.file.write_all(dst_slice).map(|()| step)
            };

            match result {
                Ok(step) => {
                    offset += step;
                    self.persisted_bytes += step as u64;
                }
                Err(e) if e.raw_os_error() == Some(libc::EINVAL) && self.direct_enabled => {
                    // Kernel rejected O_DIRECT write on this offset/device; drop O_DIRECT
                    // and complete via synchronous fdatasync.
                    clear_o_direct(self.fd)?;
                    self.direct_enabled = false;
                    self.file.write_all(&data[offset..])?;
                    self.file.sync_data()?;
                    self.persisted_bytes += (data.len() - offset) as u64;
                    fadvise_dontneed(self.fd, 0, self.persisted_bytes);
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

impl TransferSink for BlockDirectSink {
    fn write_chunk(&mut self, buf: &[u8]) -> io::Result<u64> {
        if buf.is_empty() {
            return Ok(self.persisted_bytes);
        }

        if !self.direct_enabled {
            self.file.write_all(buf)?;
            self.file.sync_data()?;
            self.persisted_bytes += buf.len() as u64;
            fadvise_dontneed(self.fd, 0, self.persisted_bytes);
            return Ok(self.persisted_bytes);
        }

        let aligned_len = buf.len() & !(DIRECT_ALIGN - 1);
        if aligned_len > 0 {
            self.write_aligned_direct(&buf[..aligned_len])?;
        }

        let tail = &buf[aligned_len..];
        if !tail.is_empty() {
            // Unaligned final chunk (< 4096 bytes): disable O_DIRECT for the tail
            // and flush immediately.
            clear_o_direct(self.fd)?;
            self.direct_enabled = false;
            self.file.write_all(tail)?;
            self.file.sync_data()?;
            self.persisted_bytes += tail.len() as u64;
            fadvise_dontneed(self.fd, 0, self.persisted_bytes);
        }

        Ok(self.persisted_bytes)
    }

    fn finalize(self: Box<Self>) -> io::Result<u64> {
        self.file.sync_all()?;
        fadvise_dontneed(self.fd, 0, self.persisted_bytes);
        blkflsbuf_best_effort(self.fd);
        Ok(self.persisted_bytes)
    }

    fn abort(self: Box<Self>) {
        // Raw block device has no temporary file to unlink; flush what was written.
        let _ = self.file.sync_all();
    }
}

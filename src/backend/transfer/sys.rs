use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// Advise the kernel that the file will be read sequentially so it uses aggressive
/// readahead without retaining unneeded pages once evicted.
pub fn fadvise_sequential(fd: RawFd) {
    unsafe {
        let _ = libc::posix_fadvise(fd, 0, 0, libc::POSIX_FADV_SEQUENTIAL);
    }
}

/// Advise the kernel to drop clean pages in `[offset, offset + len)` from the page cache.
/// Note: on Linux, `POSIX_FADV_DONTNEED` only evicts clean pages — call after
/// `sync_range_wait` or `fdatasync` on write descriptors, or immediately after reading
/// on read descriptors.
pub fn fadvise_dontneed(fd: RawFd, offset: u64, len: u64) {
    unsafe {
        let _ = libc::posix_fadvise(
            fd,
            offset as libc::off_t,
            len as libc::off_t,
            libc::POSIX_FADV_DONTNEED,
        );
    }
}

/// Initiate asynchronous writeout of dirty pages in `[offset, offset + len)` to the
/// storage device queue without blocking (`SYNC_FILE_RANGE_WRITE`).
pub fn sync_range_async(fd: RawFd, offset: u64, len: u64) -> io::Result<()> {
    let rc = unsafe {
        libc::sync_file_range(
            fd,
            offset as libc::off64_t,
            len as libc::off64_t,
            libc::SYNC_FILE_RANGE_WRITE,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Wait for all pages in `[offset, offset + len)` to finish physical writeback
/// (`SYNC_FILE_RANGE_WAIT_BEFORE | SYNC_FILE_RANGE_WRITE | SYNC_FILE_RANGE_WAIT_AFTER`).
pub fn sync_range_wait(fd: RawFd, offset: u64, len: u64) -> io::Result<()> {
    let flags = libc::SYNC_FILE_RANGE_WAIT_BEFORE
        | libc::SYNC_FILE_RANGE_WRITE
        | libc::SYNC_FILE_RANGE_WAIT_AFTER;
    let rc =
        unsafe { libc::sync_file_range(fd, offset as libc::off64_t, len as libc::off64_t, flags) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Clear `O_DIRECT` on an open file descriptor so an unaligned tail (< 4096 bytes)
/// can be written cleanly before calling `fdatasync`/`fsync`.
pub fn clear_o_direct(fd: RawFd) -> io::Result<()> {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if (flags & libc::O_DIRECT) != 0 {
            let rc = libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_DIRECT);
            if rc < 0 {
                return Err(io::Error::last_os_error());
            }
        }
    }
    Ok(())
}

/// Best-effort `BLKFLSBUF` ioctl on a block device descriptor after `fsync`.
pub fn blkflsbuf_best_effort(fd: RawFd) {
    // Linux BLKFLSBUF = _IO(0x12, 97) = 0x1261
    const BLKFLSBUF: libc::c_ulong = 0x1261;
    unsafe {
        let _ = libc::ioctl(fd, BLKFLSBUF as _, 0);
    }
}

/// Open a directory and call `fsync` on it so directory entry changes (create/rename/unlink)
/// are committed to non-volatile storage.
pub fn fsync_dir(dir: &Path) -> io::Result<()> {
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(dir)?;
    f.sync_all()
}

/// Query available free bytes for unprivileged users on the filesystem containing `path`
/// via `statvfs(2)`.
pub fn statvfs_free_bytes(path: &Path) -> Option<u64> {
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    unsafe {
        let mut st: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut st) == 0 {
            let bsize = if st.f_frsize > 0 {
                st.f_frsize as u64
            } else {
                st.f_bsize as u64
            };
            (st.f_bavail as u64).checked_mul(bsize)
        } else {
            None
        }
    }
}

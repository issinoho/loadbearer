//! Platform-specific disk I/O: unbuffered file opens, positioned read/write, and
//! a best-effort page-cache drop. Everything degrades to a portable buffered
//! path when the OS or filesystem does not cooperate.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

// ---------------------------------------------------------------------------
// Unbuffered opens
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
pub fn open_unbuffered_read(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECT)
        .open(path)
}

#[cfg(target_os = "linux")]
pub fn open_unbuffered_write(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_DIRECT)
        .open(path)
}

#[cfg(windows)]
mod win_flags {
    pub const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
    pub const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
}

#[cfg(windows)]
pub fn open_unbuffered_read(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(win_flags::FILE_FLAG_NO_BUFFERING)
        .open(path)
}

#[cfg(windows)]
pub fn open_unbuffered_write(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .custom_flags(win_flags::FILE_FLAG_NO_BUFFERING | win_flags::FILE_FLAG_WRITE_THROUGH)
        .open(path)
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn open_unbuffered_read(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "unbuffered reads not supported on this platform",
    ))
}

#[cfg(not(any(target_os = "linux", windows)))]
pub fn open_unbuffered_write(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "unbuffered writes not supported on this platform",
    ))
}

// ---------------------------------------------------------------------------
// Positioned I/O
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn pread(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(buf, offset)
}

#[cfg(unix)]
fn pwrite(file: &File, buf: &[u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.write_at(buf, offset)
}

#[cfg(windows)]
fn pread(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(buf, offset)
}

#[cfg(windows)]
fn pwrite(file: &File, buf: &[u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_write(buf, offset)
}

#[cfg(not(any(unix, windows)))]
fn pread(_f: &File, _b: &mut [u8], _o: u64) -> io::Result<usize> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no positioned read",
    ))
}

#[cfg(not(any(unix, windows)))]
fn pwrite(_f: &File, _b: &[u8], _o: u64) -> io::Result<usize> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no positioned write",
    ))
}

/// Read exactly `buf.len()` bytes starting at `offset`.
pub fn pread_exact(file: &File, mut buf: &mut [u8], mut offset: u64) -> io::Result<()> {
    while !buf.is_empty() {
        match pread(file, buf, offset)? {
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short read from scratch file",
                ));
            }
            n => {
                buf = &mut buf[n..];
                offset += n as u64;
            }
        }
    }
    Ok(())
}

/// Write all of `buf` starting at `offset`.
pub fn pwrite_all(file: &File, mut buf: &[u8], mut offset: u64) -> io::Result<()> {
    while !buf.is_empty() {
        match pwrite(file, buf, offset)? {
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "zero-length write to scratch file",
                ));
            }
            n => {
                buf = &buf[n..];
                offset += n as u64;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Cache management
// ---------------------------------------------------------------------------

/// Best-effort hint to evict the file's pages from the OS cache before a read
/// pass. No-op where the platform offers no portable mechanism.
#[cfg(target_os = "linux")]
pub fn drop_from_cache(file: &File, len: u64) {
    use std::os::unix::io::AsRawFd;
    // SAFETY: fd is valid for the lifetime of `file`; posix_fadvise has no
    // memory-safety requirements and its result is advisory.
    unsafe {
        libc::posix_fadvise(
            file.as_raw_fd(),
            0,
            len as libc::off_t,
            libc::POSIX_FADV_DONTNEED,
        );
    }
}

#[cfg(not(target_os = "linux"))]
pub fn drop_from_cache(_file: &File, _len: u64) {}

/// True when `path` lives on a RAM-backed filesystem (tmpfs / ramfs), where
/// "disk" figures actually measure memory. Linux-only; conservatively false
/// elsewhere.
#[cfg(target_os = "linux")]
pub fn is_memory_backed(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    const TMPFS_MAGIC: i64 = 0x0102_1994;
    const RAMFS_MAGIC: i64 = 0x8584_58F6;

    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `buf` is written by a successful statfs call before it is read.
    unsafe {
        let mut buf: libc::statfs = std::mem::zeroed();
        if libc::statfs(c_path.as_ptr(), &mut buf) != 0 {
            return false;
        }
        // `f_type` is `i64` on glibc/x86_64 but narrower on some targets (musl,
        // 32-bit); the cast keeps this comparison portable.
        #[allow(clippy::unnecessary_cast)]
        let ty = buf.f_type as i64;
        ty == TMPFS_MAGIC || ty == RAMFS_MAGIC
    }
}

#[cfg(not(target_os = "linux"))]
pub fn is_memory_backed(_path: &Path) -> bool {
    false
}

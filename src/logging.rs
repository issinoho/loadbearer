//! File logging for diagnostics.
//!
//! Every invocation writes a diagnostic log — by default to a per-user cache
//! location (`$XDG_CACHE_HOME/loadbearer/loadbearer.log` on Linux,
//! `%LOCALAPPDATA%\loadbearer\loadbearer.log` on Windows, the system temp dir
//! otherwise). `--log-file PATH` redirects it; `--no-log` turns it off.
//!
//! It is a hand-rolled [`log::Log`] that appends one timestamped line per
//! record and flushes immediately, so the tail survives a crash or a
//! `SIGKILL`. The file is rotated to `<name>.old` once it passes ~2 MiB, so it
//! never grows without bound. Levels: default `info`; `--log-level` or the
//! `LOADBEARER_LOG` env var (`off`/`error`/`warn`/`info`/`debug`/`trace`) tune
//! it, the flag winning.
//!
//! Nothing here is allowed to break the tool: if the log file can't be opened,
//! a one-line note goes to stderr and the run continues with logging disabled.
//!
//! Instrumentation is placed at lifecycle boundaries, fallbacks, and
//! external-resource interactions — never inside a timed measurement loop, so
//! always-on logging at the default `info` level does not perturb benchmark
//! numbers. (`debug`/`trace` add a line per subtest / per timed iteration, but
//! still only *between* measurements.)

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Where the diagnostic log should go, resolved from the CLI flags.
pub enum LogTarget {
    /// The default per-user cache path.
    Default,
    /// An explicit `--log-file PATH`.
    Path(PathBuf),
    /// `--no-log`.
    Disabled,
}

/// Roughly 2 MiB: past this on start-up the log is rotated to `<name>.old`.
const ROTATE_BYTES: u64 = 2 * 1024 * 1024;

struct FileLogger {
    sink: Mutex<File>,
    level: LevelFilter,
}

impl Log for FileLogger {
    fn enabled(&self, meta: &Metadata) -> bool {
        meta.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let ts = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "????-??-??T??:??:??Z".to_string());
        if let Ok(mut sink) = self.sink.lock() {
            // One write, one flush — a killed process keeps every line it logged.
            let _ = writeln!(
                sink,
                "{ts} {:<5} {:<28} {}",
                record.level(),
                record.target(),
                record.args(),
            );
            let _ = sink.flush();
        }
    }

    fn flush(&self) {
        if let Ok(mut sink) = self.sink.lock() {
            let _ = sink.flush();
        }
    }
}

/// Install the global logger. Call once, as early in `main` as possible.
/// Returns the path being written to, if any, for a one-line startup note.
pub fn init(target: LogTarget, level_flag: Option<LevelFilter>) -> Option<PathBuf> {
    let path = match target {
        LogTarget::Disabled => return None,
        LogTarget::Path(p) => p,
        LogTarget::Default => default_path(),
    };
    let level = level_flag
        .or_else(level_from_env)
        .unwrap_or(LevelFilter::Info);
    if level == LevelFilter::Off {
        return None;
    }

    match open(&path) {
        Ok(file) => {
            let logger = FileLogger {
                sink: Mutex::new(file),
                level,
            };
            // `set_boxed_logger` only fails if a logger is already set, which
            // can't happen here — we call this once.
            if log::set_boxed_logger(Box::new(logger)).is_ok() {
                log::set_max_level(level);
                return Some(path);
            }
            None
        }
        Err(e) => {
            eprintln!(
                "loadbearer: diagnostic log disabled ({}: {e})",
                path.display()
            );
            None
        }
    }
}

/// Open the log file for appending, rotating a large existing one aside first.
fn open(path: &Path) -> std::io::Result<File> {
    if let Some(dir) = path.parent()
        && !dir.as_os_str().is_empty()
    {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(path).map(|m| m.len()).unwrap_or(0) > ROTATE_BYTES {
        let _ = fs::rename(path, path.with_extension("log.old"));
    }
    OpenOptions::new().create(true).append(true).open(path)
}

/// `LOADBEARER_LOG=debug` etc. Unparseable values are ignored (fall through to
/// the default), rather than failing the run.
fn level_from_env() -> Option<LevelFilter> {
    std::env::var("LOADBEARER_LOG")
        .ok()?
        .parse::<LevelFilter>()
        .ok()
}

/// `<cache>/loadbearer/loadbearer.log`, falling back to the system temp dir.
fn default_path() -> PathBuf {
    let base = cache_dir().unwrap_or_else(std::env::temp_dir);
    base.join("loadbearer").join("loadbearer.log")
}

#[cfg(windows)]
fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(not(windows))]
fn cache_dir() -> Option<PathBuf> {
    if let Some(x) = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(x));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|h| PathBuf::from(h).join(".cache"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_ends_where_expected() {
        let p = default_path();
        assert!(p.ends_with("loadbearer/loadbearer.log"), "{}", p.display());
    }

    #[test]
    fn open_creates_missing_dirs_and_appends() {
        let dir = std::env::temp_dir().join(format!("lb-log-test-{}", std::process::id()));
        let path = dir.join("nested").join("t.log");
        let _ = fs::remove_dir_all(&dir);

        {
            let mut f = open(&path).unwrap();
            writeln!(f, "first").unwrap();
        }
        {
            let mut f = open(&path).unwrap();
            writeln!(f, "second").unwrap();
        }
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(body, "first\nsecond\n");

        let _ = fs::remove_dir_all(&dir);
    }
}

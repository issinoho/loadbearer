//! Emits `LOADBEARER_VERSION` — the crate version plus the git commit, build
//! date, target triple and profile — for `loadbearer --version`.

use std::process::Command;

fn main() {
    // Rebuild when the checked-out commit, the index (commit/stage), or a
    // reproducible-build date changes.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    println!("cargo:rerun-if-changed=.git/index");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    let commit = git(&["rev-parse", "--short=9", "HEAD"]).filter(|s| !s.is_empty());
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    let commit = match (commit, dirty) {
        (Some(c), true) => format!("{c}-dirty"),
        (Some(c), false) => c,
        (None, _) => "unknown".to_string(),
    };

    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".into());
    let date = build_date();

    println!(
        "cargo:rustc-env=LOADBEARER_VERSION={} ({commit} {date}, {target}, {profile})",
        env!("CARGO_PKG_VERSION")
    );
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `SOURCE_DATE_EPOCH` when set (reproducible builds), otherwise now — as `YYYY-MM-DD` UTC.
fn build_date() -> String {
    let now = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|epoch| time::OffsetDateTime::from_unix_timestamp(epoch).ok())
        .unwrap_or_else(time::OffsetDateTime::now_utc);
    let (y, m, d) = (now.year(), now.month() as u8, now.day());
    format!("{y:04}-{m:02}-{d:02}")
}

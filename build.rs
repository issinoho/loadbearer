//! Two jobs:
//!
//! 1. Emit `LOADBEARER_VERSION` — the crate version plus the git commit, build
//!    date, target triple and profile — for `loadbearer --version`.
//! 2. Generate shell completions and a man page from the CLI definition, next
//!    to the built binary, so the release workflow can bundle them.

use std::path::PathBuf;
use std::process::Command;
use std::{env, fs};

use clap::CommandFactory;
use clap_complete::Shell;

// The CLI definition, kept free of `crate::` refs so it can be included here.
#[path = "src/cli.rs"]
mod cli;

fn main() {
    stamp_version();
    generate_assets();
}

fn stamp_version() {
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

    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "unknown".into());
    let date = build_date();

    println!(
        "cargo:rustc-env=LOADBEARER_VERSION={} ({commit} {date}, {target}, {profile})",
        env!("CARGO_PKG_VERSION")
    );
}

/// Render `loadbearer.1` and the four shell completions into `OUT_DIR`, then
/// copy them next to the binary (`target/<triple>/<profile>/`) where the
/// release workflow's Package step picks them up. Best-effort: a failure here
/// must not fail the build.
fn generate_assets() {
    println!("cargo:rerun-if-changed=src/cli.rs");

    let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };

    let mut cmd = cli::Cli::command();
    cmd.set_bin_name("loadbearer");

    let mut written: Vec<PathBuf> = Vec::new();

    let mut man_buf = Vec::new();
    if clap_mangen::Man::new(cmd.clone())
        .render(&mut man_buf)
        .is_ok()
    {
        let man_path = out_dir.join("loadbearer.1");
        if fs::write(&man_path, &man_buf).is_ok() {
            written.push(man_path);
        }
    }

    for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell] {
        if let Ok(path) = clap_complete::generate_to(shell, &mut cmd, "loadbearer", &out_dir) {
            written.push(path);
        }
    }

    // OUT_DIR is <...>/target/<triple>/<profile>/build/<pkg>-<hash>/out;
    // four levels up is <...>/target/<triple>/<profile>/, next to the binary.
    if let Some(bin_dir) = out_dir.ancestors().nth(3) {
        for src in &written {
            if let Some(name) = src.file_name() {
                let _ = fs::copy(src, bin_dir.join(name));
            }
        }
    }
}

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `SOURCE_DATE_EPOCH` when set (reproducible builds), otherwise now — as `YYYY-MM-DD` UTC.
fn build_date() -> String {
    let now = env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .and_then(|epoch| time::OffsetDateTime::from_unix_timestamp(epoch).ok())
        .unwrap_or_else(time::OffsetDateTime::now_utc);
    let (y, m, d) = (now.year(), now.month() as u8, now.day());
    format!("{y:04}-{m:02}-{d:02}")
}

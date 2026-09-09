//! Preconditions for an unattended run: the checks that stop a fleet sweep
//! being disruptive or wasteful.
//!
//! A benchmark pins every core for minutes. Run across an estate on a schedule,
//! that lands on people mid-meeting, on laptops running off a dying battery
//! (where the clocks are power-capped and the numbers are worthless anyway),
//! and on machines that were measured an hour ago because a deployment tool
//! retried. These gates let the caller say "not like that".
//!
//! Two rules throughout:
//!
//! - **A blocked run is not a failure.** Deciding not to benchmark right now is
//!   the tool doing as it was told, so it exits 0. A fleet where every docked-
//!   at-lunchtime laptop shows up as a failed deployment is a fleet where
//!   somebody turns the gates off.
//! - **An unreadable signal doesn't block.** If the platform won't say whether
//!   it's on mains, the run proceeds and says so, rather than silently skipping
//!   an entire estate because a power interface returned "unknown".
//!
//! Nothing here is persisted: `--skip-if-newer-than` reads the mtime of the
//! `--output` file the previous run already wrote, so there is no state file
//! and no registry key (see PRIVACY.md).

use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

/// Global CPU load, in percent, at or above which `--if-idle` defers the run.
/// Deliberately generous: this asks "is the machine already busy", not "is
/// anyone using it".
pub const IDLE_MAX_LOAD_PCT: f32 = 20.0;

/// What the gates observed, for the result file. All of it is optional because
/// a signal is only sampled when the gate that needs it was asked for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GateReport {
    /// Whether the machine was on mains when the run started. Absent when
    /// `--not-on-battery` wasn't given, or the platform wouldn't say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_ac: Option<bool>,
    /// Global CPU load sampled immediately before the run, percent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_load_pct: Option<f64>,
    /// Random start delay actually waited, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_secs: Option<f64>,
}

impl GateReport {
    pub fn is_empty(&self) -> bool {
        *self == GateReport::default()
    }
}

/// The outcome of evaluating the gates.
#[derive(Debug)]
pub enum Decision {
    /// Go ahead; carry this into the result file.
    Proceed(Box<GateReport>),
    /// Don't run, for this reason. The caller exits 0.
    Skip(String),
}

/// Is this machine on mains?
///
/// `None` means the platform didn't give a usable answer, which is treated as
/// "don't block". A machine with no battery at all — a desktop, a server, most
/// VMs — is on mains by definition.
pub fn on_mains(battery_state: Option<&str>) -> Option<bool> {
    match battery_state {
        None => Some(true),
        Some(s) => match s.to_ascii_lowercase().as_str() {
            "discharging" => Some(false),
            "charging" | "full" => Some(true),
            // "unknown", "empty", anything new: not a usable signal.
            _ => None,
        },
    }
}

/// Is the machine quiet enough to measure?
pub fn is_idle(load_pct: f32, max_pct: f32) -> bool {
    load_pct < max_pct
}

/// Parse a `--skip-if-newer-than` age: a positive integer and a unit, one of
/// `s`, `m`, `h`, `d`. A bare number is rejected rather than guessed at — "7"
/// could reasonably mean seconds or days and getting it wrong either re-runs
/// the estate or never runs it again.
pub fn parse_age(s: &str) -> Result<Duration> {
    let t = s.trim();
    let (digits, unit) = t.split_at(t.find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len()));
    if digits.is_empty() {
        bail!("{s:?} doesn't start with a number (expected something like 7d, 12h, 30m)");
    }
    let n: u64 = digits
        .parse()
        .map_err(|_| anyhow::anyhow!("{digits:?} isn't a whole number"))?;
    if n == 0 {
        bail!("an age of zero would skip every run; leave the flag out instead");
    }
    let secs = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86_400,
        "" => bail!("{s:?} has no unit; use s, m, h or d (e.g. 7d)"),
        other => bail!("unknown unit {other:?} in {s:?}; use s, m, h or d"),
    };
    Ok(Duration::from_secs(n * secs))
}

/// How old is `path`, or `None` if it doesn't exist / can't be read.
pub fn age_of(path: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    modified.elapsed().ok()
}

/// A random start delay in `0..=max`, so an estate told to run at 09:00 doesn't
/// all hit the same file share at 09:00.
///
/// Seeded from the wall clock and the hostname, never from the workload seed —
/// that one is fixed by design so every machine runs an identical benchmark,
/// and seeding the jitter from it would give every machine an identical delay.
pub fn pick_jitter(max: Duration, hostname: Option<&str>) -> Duration {
    if max.is_zero() {
        return Duration::ZERO;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let host_hash = hostname.map_or(0u64, |h| {
        h.bytes().fold(0xcbf2_9ce4_8422_2325u64, |acc, b| {
            (acc ^ b as u64).wrapping_mul(0x1000_0000_01b3)
        })
    });
    let mut rng = crate::util::SplitMix64::new(nanos ^ host_hash);
    let millis = max.as_millis().max(1) as u64;
    Duration::from_millis(rng.below(millis + 1))
}

/// The gates a run was asked to respect.
#[derive(Debug, Default)]
pub struct Gates<'a> {
    pub not_on_battery: bool,
    pub if_idle: bool,
    pub jitter: Option<Duration>,
    /// Paired with `output`: skip when that file is newer than this.
    pub skip_if_newer_than: Option<Duration>,
    pub output: Option<&'a Path>,
    /// Owned rather than borrowed: the caller reads it straight from the OS
    /// here, before the full inventory is collected.
    pub hostname: Option<String>,
}

impl Gates<'_> {
    /// Check the gates in increasing order of cost, so a run that's going to be
    /// skipped is skipped before anything is spent on it — the recency check
    /// reads one mtime, the idle check costs ~200 ms of sampling, and the
    /// jitter sleep goes last so it's only paid by a run that will happen.
    pub fn evaluate(&self) -> Decision {
        let mut report = GateReport::default();

        if let (Some(max_age), Some(path)) = (self.skip_if_newer_than, self.output)
            && let Some(age) = age_of(path)
            && age < max_age
        {
            return Decision::Skip(format!(
                "{} was written {}s ago, inside the --skip-if-newer-than window of {}s",
                path.display(),
                age.as_secs(),
                max_age.as_secs(),
            ));
        }

        if self.not_on_battery {
            let state = crate::battery::probe().map(|b| b.state.clone());
            match on_mains(state.as_deref()) {
                Some(true) => report.on_ac = Some(true),
                Some(false) => {
                    return Decision::Skip(
                        "on battery power, and --not-on-battery was given (clocks are \
                         usually capped on battery, so the numbers wouldn't be comparable)"
                            .to_string(),
                    );
                }
                None => {
                    // Fail open, and say so rather than skipping silently.
                    log::info!(
                        target: "loadbearer::gates",
                        "--not-on-battery: power state {:?} is not a usable signal, proceeding",
                        state.as_deref().unwrap_or("-"),
                    );
                    eprintln!(
                        "note: --not-on-battery given, but this platform won't say whether \
                         it's on mains — running anyway"
                    );
                }
            }
        }

        if self.if_idle {
            let load = sample_cpu_load();
            report.cpu_load_pct = Some(load as f64);
            if !is_idle(load, IDLE_MAX_LOAD_PCT) {
                return Decision::Skip(format!(
                    "CPU load is {load:.0}%, at or above the --if-idle limit of \
                     {IDLE_MAX_LOAD_PCT:.0}%"
                ));
            }
        }

        if let Some(max) = self.jitter {
            let wait = pick_jitter(max, self.hostname.as_deref());
            report.jitter_secs = Some(wait.as_secs_f64());
            log::info!(
                target: "loadbearer::gates",
                "--jitter {}s: waiting {:.1}s before starting", max.as_secs(), wait.as_secs_f64(),
            );
            eprintln!("waiting {:.0}s (--jitter) …", wait.as_secs_f64());
            std::thread::sleep(wait);
        }

        Decision::Proceed(Box::new(report))
    }
}

/// Global CPU load as a percentage, sampled over the shortest interval sysinfo
/// will give a meaningful delta for. Two refreshes are required: the first has
/// nothing to diff against.
fn sample_cpu_load() -> f32 {
    let mut sys = sysinfo::System::new();
    sys.refresh_cpu_usage();
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    sys.refresh_cpu_usage();
    sys.global_cpu_usage()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_gates_proceeds_and_reports_nothing() {
        match Gates::default().evaluate() {
            Decision::Proceed(r) => assert!(r.is_empty()),
            Decision::Skip(w) => panic!("skipped with no gates set: {w}"),
        }
    }

    #[test]
    fn a_recent_output_file_skips_the_run() {
        let dir = std::env::temp_dir().join(format!("lb-gate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("result.json");
        std::fs::write(&f, b"{}").unwrap();

        // Just written, so inside any sane window.
        let g = Gates {
            skip_if_newer_than: Some(Duration::from_secs(3600)),
            output: Some(&f),
            ..Default::default()
        };
        assert!(matches!(g.evaluate(), Decision::Skip(_)));

        // A window shorter than the file's age proceeds. One second is enough
        // of a window to be outside once we've slept past it.
        std::thread::sleep(Duration::from_millis(1100));
        let g = Gates {
            skip_if_newer_than: Some(Duration::from_secs(1)),
            output: Some(&f),
            ..Default::default()
        };
        assert!(matches!(g.evaluate(), Decision::Proceed(_)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_output_file_does_not_skip() {
        let f = std::env::temp_dir().join("lb-gate-definitely-absent.json");
        let _ = std::fs::remove_file(&f);
        let g = Gates {
            skip_if_newer_than: Some(Duration::from_secs(86_400)),
            output: Some(&f),
            ..Default::default()
        };
        assert!(matches!(g.evaluate(), Decision::Proceed(_)));
    }

    #[test]
    fn a_machine_with_no_battery_counts_as_mains() {
        assert_eq!(on_mains(None), Some(true));
    }

    #[test]
    fn battery_states_map_to_a_verdict() {
        assert_eq!(on_mains(Some("discharging")), Some(false));
        assert_eq!(on_mains(Some("charging")), Some(true));
        assert_eq!(on_mains(Some("full")), Some(true));
        // Case as reported by the OS shouldn't matter.
        assert_eq!(on_mains(Some("Discharging")), Some(false));
    }

    #[test]
    fn an_unreadable_power_state_is_not_a_verdict() {
        assert_eq!(on_mains(Some("unknown")), None);
        assert_eq!(on_mains(Some("empty")), None);
        assert_eq!(on_mains(Some("")), None);
    }

    #[test]
    fn idle_is_a_threshold_on_load() {
        assert!(is_idle(0.0, 20.0));
        assert!(is_idle(19.9, 20.0));
        assert!(!is_idle(20.0, 20.0), "the threshold itself is not idle");
        assert!(!is_idle(97.0, 20.0));
    }

    #[test]
    fn ages_parse_with_their_unit() {
        assert_eq!(parse_age("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_age("15m").unwrap(), Duration::from_secs(900));
        assert_eq!(parse_age("12h").unwrap(), Duration::from_secs(43_200));
        assert_eq!(parse_age("7d").unwrap(), Duration::from_secs(604_800));
        assert_eq!(parse_age(" 7d ").unwrap(), Duration::from_secs(604_800));
    }

    #[test]
    fn a_bare_number_is_rejected_rather_than_guessed() {
        let e = parse_age("7").unwrap_err().to_string();
        assert!(e.contains("no unit"), "{e}");
    }

    #[test]
    fn nonsense_ages_are_rejected() {
        assert!(parse_age("").is_err());
        assert!(parse_age("d").is_err());
        assert!(parse_age("7w").is_err());
        assert!(parse_age("-1d").is_err());
        assert!(parse_age("0d").is_err(), "zero would skip every run");
    }

    #[test]
    fn jitter_stays_within_its_bound() {
        for _ in 0..200 {
            let j = pick_jitter(Duration::from_secs(5), Some("host-a"));
            assert!(j <= Duration::from_secs(5), "{j:?} exceeded the bound");
        }
        assert_eq!(pick_jitter(Duration::ZERO, None), Duration::ZERO);
    }

    /// Two machines given the same window should not pick the same delay —
    /// that's the whole point of jittering a fleet.
    #[test]
    fn jitter_differs_between_hosts() {
        let a: Vec<u128> = (0..8)
            .map(|_| pick_jitter(Duration::from_secs(600), Some("host-a")).as_millis())
            .collect();
        let b: Vec<u128> = (0..8)
            .map(|_| pick_jitter(Duration::from_secs(600), Some("host-b")).as_millis())
            .collect();
        assert_ne!(a, b);
    }

    #[test]
    fn a_missing_file_has_no_age() {
        assert!(age_of(Path::new("no-such-file-here.json")).is_none());
    }

    #[test]
    fn an_empty_report_is_omitted() {
        assert!(GateReport::default().is_empty());
        assert!(
            !GateReport {
                on_ac: Some(true),
                ..Default::default()
            }
            .is_empty()
        );
    }
}

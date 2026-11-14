//! The result's `run` block: a random ID for this run, when its benchmarks
//! started and how long they took.
//!
//! The ID is there so a collector can name one run without hashing the whole
//! document, and tell two runs apart that happen to agree on everything else.
//! It is 122 random bits from the OS, shaped as a UUID v4, and **derived from
//! nothing** — not the machine, not the time, not the results — so it can't be
//! recomputed from anything and identifies the run, never the machine. When
//! the OS won't provide randomness the ID is left out rather than made up from
//! something guessable.
//!
//! The clock starts when the benchmarks do, after the unattended-run gates:
//! `--jitter` is a deliberate wait, already recorded in `gates`, not part of
//! how long the machine was busy. It stops once the run is scored, before the
//! optional link probe and soak — the soak carries its own `duration_secs`,
//! and the TUI only probes the link after the results screen is closed, so
//! counting either would make the two output paths disagree.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunInfo {
    /// Random per-run identifier, UUID-v4-shaped, lowercase.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// When the benchmarks started, RFC 3339 UTC. `timestamp` is when the
    /// document was assembled, at the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    /// Wall-clock seconds from `started_at` until the run was scored,
    /// measured on a monotonic clock so a clock change mid-run can't skew it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f64>,
    /// What this run was told to collect beyond the default inventory
    /// (`--collect` and the config's `collect`). Omitted when nothing was,
    /// so a block or field from the list that is missing means "asked, and
    /// couldn't be read", and one not on the list means "not asked".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collect: Vec<crate::optin::Item>,
    /// Whether this run had administrator / root rights. Only with
    /// `--collect elevation`, and absent if the query failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevated: Option<bool>,
}

/// What the invocation opted into, and the one opt-in fact about the run
/// rather than the machine. Required by [`RunClock::finish`], so neither
/// output path can forget it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Invocation {
    pub collect: Vec<crate::optin::Item>,
    pub elevated: Option<bool>,
}

impl From<&crate::optin::Gathered> for Invocation {
    fn from(g: &crate::optin::Gathered) -> Self {
        Invocation {
            collect: g.asked.items(),
            elevated: g.elevated,
        }
    }
}

impl RunInfo {
    /// A block holding only what the invocation opted into, for a document with
    /// no benchmark run to time or name (`soak`). Empty when nothing was asked.
    pub fn of_invocation(invocation: &Invocation) -> Self {
        RunInfo {
            collect: invocation.collect.clone(),
            elevated: invocation.elevated,
            ..RunInfo::default()
        }
    }

    pub fn is_empty(&self) -> bool {
        *self == RunInfo::default()
    }

    /// "took 3m 12s", for the line that describes how the run was done.
    pub fn took(&self) -> Option<String> {
        let secs = self.duration_secs?;
        if !secs.is_finite() || secs < 0.0 {
            return None;
        }
        let whole = secs.round() as u64;
        Some(match whole {
            0..60 => format!("took {whole}s"),
            60..3600 => format!("took {}m {:02}s", whole / 60, whole % 60),
            _ => format!("took {}h {:02}m", whole / 3600, (whole % 3600) / 60),
        })
    }
}

/// Started as the benchmarks begin; [`RunClock::finish`] turns it into the
/// block. The only way to make a [`RunInfo`] for a real run, so both output
/// paths measure the same span.
pub struct RunClock {
    id: Option<String>,
    started: Instant,
    started_at: OffsetDateTime,
}

impl RunClock {
    pub fn start() -> Self {
        let id = new_id();
        log::debug!(
            target: "loadbearer::run",
            "run id {}",
            if id.is_some() { "generated" } else { "unavailable (no OS randomness)" },
        );
        RunClock {
            id,
            started: Instant::now(),
            started_at: OffsetDateTime::now_utc(),
        }
    }

    pub fn finish(&self, invocation: &Invocation) -> RunInfo {
        let elapsed = self.started.elapsed().as_secs_f64();
        RunInfo {
            id: self.id.clone(),
            started_at: self.started_at.format(&Rfc3339).ok(),
            // Milliseconds are already finer than anything this measures.
            duration_secs: Some((elapsed * 1000.0).round() / 1000.0),
            collect: invocation.collect.clone(),
            elevated: invocation.elevated,
        }
    }
}

/// A fresh random ID, or `None` if the OS source fails — never a fallback
/// built from the time or the machine, which would make it guessable.
fn new_id() -> Option<String> {
    let mut bytes = [0u8; 16];
    match getrandom::fill(&mut bytes) {
        Ok(()) => Some(uuid_v4(bytes)),
        Err(e) => {
            log::warn!(target: "loadbearer::run", "OS randomness unavailable, run id omitted: {e}");
            None
        }
    }
}

/// Format 16 random bytes as an RFC 9562 version-4 UUID: the version and
/// variant bits are fixed, which is what lets a database column typed as UUID
/// take it.
fn uuid_v4(mut b: [u8; 16]) -> String {
    b[6] = (b[6] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn well_formed(id: &str) -> bool {
        let groups: Vec<&str> = id.split('-').collect();
        groups.iter().map(|g| g.len()).collect::<Vec<_>>() == [8, 4, 4, 4, 12]
            && id
                .chars()
                .all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c))
            && groups[2].starts_with('4')
            && matches!(groups[3].chars().next(), Some('8' | '9' | 'a' | 'b'))
    }

    #[test]
    fn version_and_variant_bits_are_set() {
        assert_eq!(uuid_v4([0; 16]), "00000000-0000-4000-8000-000000000000");
        assert_eq!(uuid_v4([0xFF; 16]), "ffffffff-ffff-4fff-bfff-ffffffffffff");
        assert!(well_formed(&uuid_v4([0xA5; 16])));
    }

    // Uses the OS randomness source, which is no hardware or network access
    // and is available in a build sandbox.
    #[test]
    fn each_run_gets_a_different_well_formed_id() {
        let a = RunClock::start().finish(&Invocation::default());
        let b = RunClock::start().finish(&Invocation::default());
        let (a_id, b_id) = (a.id.unwrap(), b.id.unwrap());
        assert!(well_formed(&a_id));
        assert!(well_formed(&b_id));
        assert_ne!(a_id, b_id);
    }

    #[test]
    fn the_block_has_a_start_and_a_non_negative_duration() {
        let run = RunClock::start().finish(&Invocation::default());
        let started = run.started_at.as_deref().unwrap();
        // `time` is built without its parser; the shape is enough here.
        let b = started.as_bytes();
        assert!(b.len() >= 20, "{started}");
        assert!(b[..4].iter().all(u8::is_ascii_digit), "{started}");
        assert_eq!(
            (b[4], b[7], b[10], b[13], b[16]),
            (b'-', b'-', b'T', b':', b':')
        );
        assert!(started.ends_with('Z'), "UTC: {started}");
        assert!(run.duration_secs.unwrap() >= 0.0);
    }

    #[test]
    fn json_shape() {
        let run = RunInfo {
            id: Some("0f8fad5b-d9cb-469f-a165-70867728950e".into()),
            started_at: Some("2026-11-13T09:00:00Z".into()),
            duration_secs: Some(192.5),
            ..Default::default()
        };
        let v = serde_json::to_value(&run).unwrap();
        assert_eq!(v["id"], "0f8fad5b-d9cb-469f-a165-70867728950e");
        assert_eq!(v["started_at"], "2026-11-13T09:00:00Z");
        assert_eq!(v["duration_secs"], 192.5);
        // Absent stays absent, not null.
        let partial = RunInfo { id: None, ..run };
        let text = serde_json::to_string(&partial).unwrap();
        assert!(!text.contains("\"id\""), "{text}");
        assert!(RunInfo::default().is_empty());
        // Nothing opted into: neither key is written.
        assert!(!text.contains("collect") && !text.contains("elevated"));
    }

    #[test]
    fn the_opt_ins_are_recorded_on_the_run() {
        use crate::optin::Item;
        let invocation = Invocation {
            collect: vec![Item::Elevation, Item::Security],
            elevated: Some(false),
        };
        let run = RunClock::start().finish(&invocation);
        let v = serde_json::to_value(&run).unwrap();
        assert_eq!(v["collect"], serde_json::json!(["elevation", "security"]));
        assert_eq!(v["elevated"], false);
        // Asked for elevation and the query failed: absent, not false.
        let failed = Invocation {
            collect: vec![Item::Elevation],
            elevated: None,
        };
        let v = serde_json::to_value(RunClock::start().finish(&failed)).unwrap();
        assert_eq!(v["collect"], serde_json::json!(["elevation"]));
        assert!(v.get("elevated").is_none());
    }

    #[test]
    fn took_reads_like_a_person_would_say_it() {
        let t = |s: f64| {
            RunInfo {
                duration_secs: Some(s),
                ..Default::default()
            }
            .took()
        };
        assert_eq!(t(42.4).as_deref(), Some("took 42s"));
        assert_eq!(t(192.0).as_deref(), Some("took 3m 12s"));
        assert_eq!(t(3725.0).as_deref(), Some("took 1h 02m"));
        assert_eq!(t(-1.0), None);
        assert_eq!(RunInfo::default().took(), None);
    }
}

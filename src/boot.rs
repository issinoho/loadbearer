//! `machine.boot`, collected only with `--collect uptime`: when the machine
//! last booted and how long it had been up when the inventory was taken.
//!
//! Read through `sysinfo`, which is already linked:
//!
//! - **Windows** — `GetTickCount64`, milliseconds since the kernel started.
//!   That counts time asleep and hibernated, and with Fast Startup (the
//!   default) "Shut down" hibernates the kernel rather than stopping it, so
//!   the number runs on across shutdowns and only a *restart* resets it. It is
//!   how long this kernel session has existed, which is what Task Manager
//!   shows as "Up time", not how long the machine has been switched on.
//! - **Linux** — `/proc/uptime` (falling back to the `sysinfo(2)` call):
//!   seconds since boot, including time suspended.
//!
//! `booted_at` is derived from the wall clock minus that count, two clocks
//! read a moment apart, so it is rounded to the second; a wall clock that is
//! wrong makes it wrong by the same amount.

use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Boot {
    /// When the machine last booted, RFC 3339 UTC, to the second.
    pub booted_at: String,
    /// Seconds it had been up when the inventory was collected.
    pub uptime_secs: u64,
}

impl Boot {
    /// "up 3d 4h, since 2026-11-10 08:12 UTC".
    pub fn summary(&self) -> String {
        let since = self.booted_at.replacen('T', " ", 1);
        let since = since.trim_end_matches('Z');
        // Drop the seconds: "since" reads to the minute.
        let since = since.get(..16).unwrap_or(since);
        format!(
            "up {}, since {since} UTC",
            crate::optin::span(self.uptime_secs)
        )
    }
}

/// Only [`crate::optin::gather`] calls this, and only when `uptime` was asked
/// for.
pub(crate) fn read() -> Option<Boot> {
    from_uptime(OffsetDateTime::now_utc(), sysinfo::System::uptime())
}

/// The block from "now" and seconds of uptime. `sysinfo` reports 0 when it
/// couldn't read the count, and no machine running this has been up for no
/// time at all, so 0 is "unknown".
fn from_uptime(now: OffsetDateTime, uptime_secs: u64) -> Option<Boot> {
    if uptime_secs == 0 {
        return None;
    }
    let now = now.replace_nanosecond(0).ok()?;
    let secs = i64::try_from(uptime_secs).ok()?;
    let booted = now.checked_sub(Duration::seconds(secs))?;
    Some(Boot {
        booted_at: booted.format(&Rfc3339).ok()?,
        uptime_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(unix: i64, nanos: u32) -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(unix)
            .unwrap()
            .replace_nanosecond(nanos)
            .unwrap()
    }

    #[test]
    fn booted_at_is_now_minus_uptime_to_the_second() {
        // 2026-11-13T12:00:00.750Z, up 3h 30m 15s.
        let b = from_uptime(at(1_794_571_200, 750_000_000), 12_615).unwrap();
        assert_eq!(b.booted_at, "2026-11-13T08:29:45Z");
        assert_eq!(b.uptime_secs, 12_615);
        assert_eq!(b.summary(), "up 3h 30m, since 2026-11-13 08:29 UTC");
    }

    #[test]
    fn an_unread_count_is_no_block() {
        assert_eq!(from_uptime(at(1_794_571_200, 0), 0), None);
        // An absurd count can't produce a date; that's no block either.
        assert_eq!(from_uptime(at(0, 0), u64::MAX), None);
    }

    #[test]
    fn json_shape() {
        let b = from_uptime(at(1_794_571_200, 0), 86_400 * 3).unwrap();
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["booted_at"], "2026-11-10T12:00:00Z");
        assert_eq!(v["uptime_secs"], 259_200);
    }
}

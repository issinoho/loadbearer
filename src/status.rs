//! The run status record: what a skipped `run` tells the fleet server, so a
//! machine that keeps being skipped can be told from one whose collector has
//! stopped. Both otherwise look stale.
//!
//! The rules, which PRIVACY.md states to users and the tests here pin:
//!
//! - **Only with an upload target.** No `--upload` / `[upload]`, no record:
//!   nothing is read, built or sent, and a skip stays the non-event it was.
//! - **Only for a skip that says something a result doesn't.** A battery or
//!   busy skip sends one; `--skip-if-newer-than` never does, because a recent
//!   result exists and says everything.
//! - **Attribution and nothing else.** The hostname, the identity block and
//!   the tags: what the fleet needs to file the record under the same machine
//!   as its results and to apply a site-scoped token (a document whose tags
//!   do not match the token's scope is refused). No inventory, no power
//!   state, nothing opted into with `--collect`.
//! - **A skip is not a failure.** Whatever happens to the post, the caller
//!   exits 0; [`Reported::Failed`] is one warning line, never an exit code.
//!   Exit 4 keeps meaning a completed run whose result could not be delivered.
//!
//! The steps that cost something (waiting out `--jitter`, reading the
//! identity, the network) are handed in, so a test can prove they are not
//! taken when no record is due.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::gates::Skip;
use crate::identity::MachineId;
use crate::tags::Tags;
use crate::upload::Target;

/// The document's `schema`. Its own string, so no consumer can mistake a
/// status record for a result.
pub const SCHEMA: &str = "loadbearer.run-status/1";

/// The one status so far. A string rather than a type so a later value needs
/// no schema bump.
pub const STATUS_SKIPPED: &str = "skipped";

/// A run that did not happen, and why. Absent data is left out, not guessed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStatus {
    pub schema: String,
    pub tool_version: String,
    /// When the record was made: what makes two records from one machine
    /// distinct. There is no run ID, because nothing ran.
    pub timestamp: String,
    pub status: String,
    /// A short stable word: `on_battery` or `busy`.
    pub reason: String,
    /// The sentence the skip printed.
    pub detail: String,
    /// What a result carries as `machine.hostname`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// What a result carries as `machine.identity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<MachineId>,
    /// What a result carries as `tags`.
    #[serde(default, skip_serializing_if = "Tags::is_empty")]
    pub tags: Tags,
}

impl RunStatus {
    /// The record for a skip, or `None` for a skip that sends none.
    pub fn for_skip(
        skip: &Skip,
        hostname: Option<String>,
        identity: Option<MachineId>,
        tags: &Tags,
    ) -> Option<Self> {
        let reason = skip.reason.status_word()?;
        Some(Self {
            schema: SCHEMA.to_string(),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_default(),
            status: STATUS_SKIPPED.to_string(),
            reason: reason.to_string(),
            detail: skip.detail.clone(),
            hostname,
            identity: identity.filter(|id| !id.is_empty()),
            tags: tags.clone(),
        })
    }
}

/// Why no record was sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotDue {
    /// No upload target is configured.
    NoUpload,
    /// `--no-status`, or `status = false` under `[upload]`.
    TurnedOff,
    /// A skip that sends none (`--skip-if-newer-than`).
    ThisReason,
}

impl NotDue {
    /// For the diagnostic log.
    pub fn why(self) -> &'static str {
        match self {
            Self::NoUpload => "no upload target is configured",
            Self::TurnedOff => "turned off with --no-status or status = false",
            Self::ThisReason => "this kind of skip sends no record",
        }
    }
}

/// What came of reporting a skip.
#[derive(Debug)]
pub enum Reported {
    NotDue(NotDue),
    /// The server answered `recorded`.
    Sent,
    /// Built and posted, and not recorded; the text is the one warning.
    Failed(anyhow::Error),
}

/// The costly steps, injected.
pub struct Steps<'a> {
    /// Waits out `--jitter`, if it was asked for. Called once, and only when
    /// a record is due, so a fleet skipping together does not post together.
    pub wait: &'a dyn Fn(),
    /// Reads the identity block.
    pub identity: &'a dyn Fn() -> Option<MachineId>,
    /// Posts the serialised record.
    pub send: &'a dyn Fn(&Target, &str) -> Result<()>,
}

/// Report a skip, if one is due: decide, then wait, read the identity, build,
/// serialise and post once. The order matters: the decision is made from
/// what is already known, so a machine that will send nothing reads nothing.
pub fn report(
    skip: &Skip,
    target: Option<&Target>,
    enabled: bool,
    hostname: Option<String>,
    tags: &Tags,
    steps: &Steps<'_>,
) -> Reported {
    let Some(target) = target else {
        return Reported::NotDue(NotDue::NoUpload);
    };
    if !enabled {
        return Reported::NotDue(NotDue::TurnedOff);
    }
    if skip.reason.status_word().is_none() {
        return Reported::NotDue(NotDue::ThisReason);
    }
    (steps.wait)();
    let Some(record) = RunStatus::for_skip(skip, hostname, (steps.identity)(), tags) else {
        return Reported::NotDue(NotDue::ThisReason);
    };
    let body = match serde_json::to_string(&record) {
        Ok(body) => body,
        Err(e) => return Reported::Failed(e.into()),
    };
    match (steps.send)(target, &body) {
        Ok(()) => Reported::Sent,
        Err(e) => Reported::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gates::SkipReason;
    use crate::upload::test_support::{reply, server, target};
    use std::cell::Cell;
    use std::time::{Duration, Instant};

    fn skip(reason: SkipReason) -> Skip {
        Skip {
            reason,
            detail: "a synthetic detail".to_string(),
        }
    }

    fn identity() -> MachineId {
        MachineId {
            machine_id: Some("synthetic-machine-id".into()),
            smbios_uuid: Some("synthetic-uuid".into()),
            serial: Some("synthetic-serial".into()),
            asset_tag: Some("synthetic-asset".into()),
        }
    }

    fn tags() -> Tags {
        Tags::from([("site".to_string(), "glasgow".to_string())])
    }

    /// Counts what `report` did, and answers every post as `answer` says.
    #[derive(Default)]
    struct Spy {
        waits: Cell<u32>,
        identity_reads: Cell<u32>,
        posts: std::cell::RefCell<Vec<String>>,
        fail: bool,
    }

    impl Spy {
        fn report(&self, skip: &Skip, target: Option<&Target>, enabled: bool) -> Reported {
            let wait = || self.waits.set(self.waits.get() + 1);
            let read = || {
                self.identity_reads.set(self.identity_reads.get() + 1);
                Some(identity())
            };
            let send = |_: &Target, body: &str| -> Result<()> {
                self.posts.borrow_mut().push(body.to_string());
                if self.fail {
                    anyhow::bail!("synthetic failure")
                }
                Ok(())
            };
            report(
                skip,
                target,
                enabled,
                Some("PC-0142".into()),
                &tags(),
                &Steps {
                    wait: &wait,
                    identity: &read,
                    send: &send,
                },
            )
        }

        fn touched_nothing(&self) -> bool {
            self.waits.get() == 0
                && self.identity_reads.get() == 0
                && self.posts.borrow().is_empty()
        }
    }

    #[test]
    fn with_no_upload_configured_nothing_is_waited_for_read_built_or_sent() {
        let spy = Spy::default();
        for reason in [SkipReason::OnBattery, SkipReason::Busy] {
            let out = spy.report(&skip(reason), None, true);
            assert!(matches!(out, Reported::NotDue(NotDue::NoUpload)), "{out:?}");
        }
        assert!(spy.touched_nothing());
    }

    #[test]
    fn a_recent_result_skip_sends_nothing_even_with_an_upload() {
        let spy = Spy::default();
        let t = target("https://fleet.example/api/upload");
        let out = spy.report(&skip(SkipReason::RecentResult), Some(&t), true);
        assert!(
            matches!(out, Reported::NotDue(NotDue::ThisReason)),
            "{out:?}"
        );
        assert!(spy.touched_nothing());
    }

    #[test]
    fn turned_off_sends_nothing() {
        let spy = Spy::default();
        let t = target("https://fleet.example/api/upload");
        for reason in [SkipReason::OnBattery, SkipReason::Busy] {
            let out = spy.report(&skip(reason), Some(&t), false);
            assert!(
                matches!(out, Reported::NotDue(NotDue::TurnedOff)),
                "{out:?}"
            );
        }
        assert!(spy.touched_nothing());
    }

    #[test]
    fn a_due_record_waits_reads_the_identity_once_and_posts_once() {
        let spy = Spy::default();
        let t = target("https://fleet.example/api/upload");
        for (n, reason) in [SkipReason::OnBattery, SkipReason::Busy]
            .into_iter()
            .enumerate()
        {
            let out = spy.report(&skip(reason), Some(&t), true);
            assert!(matches!(out, Reported::Sent), "{out:?}");
            let n = n as u32 + 1;
            assert_eq!(
                (spy.waits.get(), spy.identity_reads.get()),
                (n, n),
                "once each per skip"
            );
            assert_eq!(spy.posts.borrow().len(), n as usize);
        }
    }

    #[test]
    fn the_document_has_exactly_the_agreed_fields() {
        let spy = Spy::default();
        let t = target("https://fleet.example/api/upload");
        spy.report(&skip(SkipReason::OnBattery), Some(&t), true);
        let body = spy.posts.borrow()[0].clone();
        let doc: serde_json::Value = serde_json::from_str(&body).unwrap();
        let keys: Vec<&str> = doc
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let mut want = vec![
            "schema",
            "tool_version",
            "timestamp",
            "status",
            "reason",
            "detail",
            "hostname",
            "identity",
            "tags",
        ];
        let mut got = keys.clone();
        want.sort_unstable();
        got.sort_unstable();
        assert_eq!(got, want, "no inventory, power, collect or run fields");

        assert_eq!(doc["schema"], "loadbearer.run-status/1");
        assert_eq!(doc["status"], "skipped");
        assert_eq!(doc["reason"], "on_battery");
        assert_eq!(doc["detail"], "a synthetic detail");
        assert_eq!(doc["tool_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(doc["hostname"], "PC-0142");
        assert_eq!(doc["tags"]["site"], "glasgow");
        let stamp = doc["timestamp"].as_str().unwrap();
        assert!(
            stamp.len() >= 20 && stamp.contains('T') && stamp.ends_with('Z'),
            "an RFC 3339 UTC stamp"
        );

        let mut id: Vec<&str> = doc["identity"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        id.sort_unstable();
        assert_eq!(id, ["asset_tag", "machine_id", "serial", "smbios_uuid"]);

        // The identity block is the one a result carries, byte for byte.
        assert_eq!(doc["identity"], serde_json::to_value(identity()).unwrap());
    }

    #[test]
    fn the_busy_reason_word_and_unreadable_fields() {
        let record = RunStatus::for_skip(
            &skip(SkipReason::Busy),
            None,
            Some(MachineId::default()),
            &Tags::new(),
        )
        .unwrap();
        let doc = serde_json::to_value(&record).unwrap();
        assert_eq!(doc["reason"], "busy");
        // Unreadable or empty is omitted, never guessed or blank.
        for absent in ["hostname", "identity", "tags"] {
            assert!(doc.get(absent).is_none(), "{absent} should be omitted");
        }
    }

    #[test]
    fn a_skip_with_no_status_word_builds_no_record() {
        assert!(
            RunStatus::for_skip(&skip(SkipReason::RecentResult), None, None, &Tags::new())
                .is_none()
        );
    }

    #[test]
    fn a_failed_post_is_a_failed_report_not_a_panic() {
        let spy = Spy {
            fail: true,
            ..Default::default()
        };
        let t = target("https://fleet.example/api/upload");
        let out = spy.report(&skip(SkipReason::Busy), Some(&t), true);
        assert!(matches!(out, Reported::Failed(_)), "{out:?}");
        assert_eq!(spy.posts.borrow().len(), 1, "one attempt here, no retry");
    }

    /// The real send against a loopback server, for the wire format.
    fn report_to(url: &str, reason: SkipReason) -> Reported {
        let t = target(url);
        report(
            &skip(reason),
            Some(&t),
            true,
            Some("PC-0142".into()),
            &tags(),
            &Steps {
                wait: &|| {},
                identity: &|| Some(identity()),
                send: &crate::upload::send_status,
            },
        )
    }

    #[test]
    fn battery_and_busy_skips_each_post_once_with_the_bearer_token() {
        for (reason, word) in [
            (SkipReason::OnBattery, "on_battery"),
            (SkipReason::Busy, "busy"),
        ] {
            let (url, seen) = server(vec![reply("200 OK", "", r#"{"outcome":"recorded"}"#)]);
            let out = report_to(&url, reason);
            assert!(matches!(out, Reported::Sent), "{out:?}");

            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 1);
            let request = seen[0].to_ascii_lowercase();
            assert!(
                request.starts_with("post /api/upload "),
                "wrong request line"
            );
            assert!(
                request.contains("authorization: bearer lbf_s3cret"),
                "token header missing"
            );
            assert!(request.contains("content-type: application/json"));
            let (_, body) = seen[0].split_once("\r\n\r\n").unwrap();
            let doc: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(doc["schema"], SCHEMA);
            assert_eq!(doc["reason"], word);
            // Present, so a token scoped to this site would accept it.
            assert_eq!(doc["tags"]["site"], "glasgow");
        }
    }

    #[test]
    fn an_older_server_answering_400_is_one_failure_that_names_the_cause() {
        let (url, seen) = server(vec![reply(
            "400 Bad Request",
            "",
            r#"{"error":"not a loadbearer result"}"#,
        )]);
        let Reported::Failed(e) = report_to(&url, SkipReason::Busy) else {
            panic!("a 400 must not read as recorded");
        };
        let text = format!("{e:#}");
        assert!(
            text.contains("HTTP 400") && text.contains("older"),
            "{text}"
        );
        assert!(!text.contains("lbf_s3cret"));
        assert_eq!(seen.lock().unwrap().len(), 1, "no retry");
    }

    #[test]
    fn a_server_error_is_not_retried() {
        let (url, seen) = server(vec![
            reply("503 Service Unavailable", "Retry-After: 1\r\n", "later"),
            reply("200 OK", "", r#"{"outcome":"recorded"}"#),
        ]);
        let started = Instant::now();
        assert!(matches!(
            report_to(&url, SkipReason::Busy),
            Reported::Failed(_)
        ));
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_200_that_does_not_say_recorded_is_not_a_success() {
        let (url, _) = server(vec![reply("200 OK", "", r#"{"outcome":"added"}"#)]);
        assert!(matches!(
            report_to(&url, SkipReason::Busy),
            Reported::Failed(_)
        ));
    }

    #[test]
    fn an_unreachable_server_fails_once_and_quickly() {
        // Bound then dropped: nothing is listening there any more.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let started = Instant::now();
        let out = report_to(
            &format!("http://127.0.0.1:{port}/api/upload"),
            SkipReason::Busy,
        );
        assert!(matches!(out, Reported::Failed(_)), "{out:?}");
        // The result path would back off 2 s then 8 s; a skip does not.
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}

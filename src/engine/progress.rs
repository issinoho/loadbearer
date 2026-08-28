//! Progress reporting. The engine emits [`ProgressEvent`]s; a [`Progress`] sink
//! renders them. Phase 2 ships only the plain stderr sink; the TUI adds its own.

use std::io::{IsTerminal, Write};

use super::SubtestOutcome;

// Several fields (subtest `id`, `timed_runs`) are only read by the TUI sink,
// which lands in a later phase; the plain sink ignores them.
#[allow(dead_code)]
pub enum ProgressEvent<'a> {
    BenchStart {
        id: &'a str,
        label: &'a str,
        subtests: usize,
    },
    SubtestStart {
        id: &'a str,
        label: &'a str,
        timed_runs: u32,
    },
    Warmup {
        id: &'a str,
    },
    Run {
        id: &'a str,
        run: u32,
        of: u32,
        value: f64,
        unit: &'a str,
    },
    SubtestDone {
        id: &'a str,
        outcome: &'a SubtestOutcome,
    },
    BenchDone {
        id: &'a str,
    },
}

pub trait Progress {
    fn on_event(&mut self, event: ProgressEvent<'_>);
}

/// Discards every event. Handy for tests and `--json` runs.
#[allow(dead_code)]
pub struct SilentProgress;

impl Progress for SilentProgress {
    fn on_event(&mut self, _event: ProgressEvent<'_>) {}
}

/// Line-oriented progress on stderr, so stdout stays clean for JSON.
///
/// On an interactive stderr the active run line is rewritten in place with `\r`;
/// otherwise every update is its own newline-terminated line so captured or
/// piped logs stay readable.
pub struct PlainProgress {
    current_label: String,
    interactive: bool,
}

impl PlainProgress {
    pub fn new() -> Self {
        Self {
            current_label: String::new(),
            interactive: std::io::stderr().is_terminal(),
        }
    }
}

impl Default for PlainProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress for PlainProgress {
    fn on_event(&mut self, event: ProgressEvent<'_>) {
        let mut err = std::io::stderr();
        match event {
            ProgressEvent::BenchStart {
                label, subtests, ..
            } => {
                let _ = writeln!(err, "\n{label}  ({subtests} subtests)");
            }
            ProgressEvent::SubtestStart { label, .. } => {
                self.current_label = label.to_string();
                if self.interactive {
                    let _ = write!(err, "  {label:<26} warming up…");
                    let _ = err.flush();
                }
            }
            ProgressEvent::Warmup { .. } => {}
            ProgressEvent::Run {
                run,
                of,
                value,
                unit,
                ..
            } => {
                if self.interactive {
                    let _ = write!(
                        err,
                        "\r  {:<26} run {run}/{of}  {value:>12.1} {unit}            ",
                        self.current_label
                    );
                    let _ = err.flush();
                } else {
                    let _ = writeln!(
                        err,
                        "  {:<26} run {run}/{of}  {value:>12.1} {unit}",
                        self.current_label
                    );
                }
            }
            ProgressEvent::SubtestDone { outcome, .. } => {
                let spread = if outcome.value.abs() > f64::EPSILON {
                    (outcome.stats.max - outcome.stats.min) / outcome.value * 100.0
                } else {
                    0.0
                };
                if self.interactive {
                    let _ = writeln!(
                        err,
                        "\r  {:<26} {:>12.1} {:<9} ±{:>5.1}%  {:<6}          ",
                        outcome.label,
                        outcome.value,
                        outcome.unit,
                        spread,
                        outcome.confidence.as_str(),
                    );
                } else {
                    let _ = writeln!(
                        err,
                        "  {:<26} = {:>12.1} {:<9} ±{:.1}%  {}",
                        outcome.label,
                        outcome.value,
                        outcome.unit,
                        spread,
                        outcome.confidence.as_str(),
                    );
                }
            }
            ProgressEvent::BenchDone { .. } => {}
        }
    }
}

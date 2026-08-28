//! TUI state: a mirror of the run's progress that the draw code renders, updated
//! from messages sent by the worker thread.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use crate::engine::SubtestOutcome;
use crate::scoring::ResultFile;

/// A message from the worker thread to the UI loop.
pub enum Msg {
    Progress(Ev),
    Done(Box<ResultFile>),
    Failed(String),
}

/// An owned, sendable form of the engine's borrowed `ProgressEvent`.
pub enum Ev {
    BenchStart,
    SubtestStart {
        bench: usize,
        sub: usize,
        timed: u32,
    },
    Warmup {
        bench: usize,
        sub: usize,
    },
    Run {
        bench: usize,
        sub: usize,
        run: u32,
        value: f64,
    },
    SubtestDone {
        bench: usize,
        sub: usize,
        outcome: Box<SubtestOutcome>,
    },
    BenchDone {
        bench: usize,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SubState {
    Pending,
    Warmup,
    Running,
    Done,
}

pub struct SubRow {
    pub label: String,
    pub unit: String,
    pub timed: u32,
    pub runs_done: u32,
    pub state: SubState,
    pub last_value: Option<f64>,
    pub outcome: Option<SubtestOutcome>,
}

impl SubRow {
    pub fn fraction(&self) -> f64 {
        match self.state {
            SubState::Pending => 0.0,
            SubState::Warmup => 0.05,
            SubState::Running if self.timed > 0 => {
                (self.runs_done as f64 / self.timed as f64).clamp(0.05, 1.0)
            }
            SubState::Running => 0.5,
            SubState::Done => 1.0,
        }
    }

    /// The value to show: final median once done, else the latest run.
    pub fn display_value(&self) -> Option<f64> {
        self.outcome.as_ref().map(|o| o.value).or(self.last_value)
    }
}

pub struct BenchRow {
    pub label: String,
    pub subs: Vec<SubRow>,
    pub done: bool,
}

pub enum Phase {
    Running,
    Done(Box<ResultFile>),
    Failed(String),
}

pub struct App {
    pub header: String,
    pub started: Instant,
    pub benches: Vec<BenchRow>,
    pub phase: Phase,
    pub abort: Arc<AtomicBool>,
    pub cancelling: bool,
    pub results_scroll: u16,
}

impl App {
    pub fn new(
        header: String,
        specs: Vec<(String, Vec<(String, String)>)>,
        abort: Arc<AtomicBool>,
    ) -> Self {
        let benches = specs
            .into_iter()
            .map(|(label, subs)| BenchRow {
                label,
                subs: subs
                    .into_iter()
                    .map(|(label, unit)| SubRow {
                        label,
                        unit,
                        timed: 0,
                        runs_done: 0,
                        state: SubState::Pending,
                        last_value: None,
                        outcome: None,
                    })
                    .collect(),
                done: false,
            })
            .collect();
        Self {
            header,
            started: Instant::now(),
            benches,
            phase: Phase::Running,
            abort,
            cancelling: false,
            results_scroll: 0,
        }
    }

    pub fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Done(result) => self.phase = Phase::Done(result),
            Msg::Failed(err) => self.phase = Phase::Failed(err),
            Msg::Progress(ev) => self.apply_ev(ev),
        }
    }

    fn apply_ev(&mut self, ev: Ev) {
        match ev {
            Ev::BenchStart => {}
            Ev::SubtestStart { bench, sub, timed } => {
                if let Some(row) = self.sub_mut(bench, sub) {
                    row.timed = timed;
                    row.state = SubState::Warmup;
                }
            }
            Ev::Warmup { bench, sub } => {
                if let Some(row) = self.sub_mut(bench, sub) {
                    row.state = SubState::Warmup;
                }
            }
            Ev::Run {
                bench,
                sub,
                run,
                value,
            } => {
                if let Some(row) = self.sub_mut(bench, sub) {
                    row.state = SubState::Running;
                    row.runs_done = run;
                    row.last_value = Some(value);
                }
            }
            Ev::SubtestDone {
                bench,
                sub,
                outcome,
            } => {
                if let Some(row) = self.sub_mut(bench, sub) {
                    row.state = SubState::Done;
                    row.runs_done = row.timed;
                    row.outcome = Some(*outcome);
                }
            }
            Ev::BenchDone { bench } => {
                if let Some(b) = self.benches.get_mut(bench) {
                    b.done = true;
                }
            }
        }
    }

    fn sub_mut(&mut self, bench: usize, sub: usize) -> Option<&mut SubRow> {
        self.benches.get_mut(bench)?.subs.get_mut(sub)
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.phase, Phase::Done(_) | Phase::Failed(_))
    }

    /// Overall completion fraction across every subtest.
    pub fn progress_fraction(&self) -> f64 {
        let total: usize = self.benches.iter().map(|b| b.subs.len()).sum();
        if total == 0 {
            return 0.0;
        }
        let done: f64 = self
            .benches
            .iter()
            .flat_map(|b| &b.subs)
            .map(SubRow::fraction)
            .sum();
        done / total as f64
    }

    /// Index of the first not-yet-finished subtest, as a flat row offset that
    /// includes one line per benchmark header.
    pub fn active_line(&self) -> usize {
        let mut line = 0;
        for b in &self.benches {
            line += 1; // header
            for s in &b.subs {
                if s.state != SubState::Done {
                    return line;
                }
                line += 1;
            }
        }
        line
    }

    pub fn scroll_results(&mut self, delta: i32) {
        let next = self.results_scroll as i32 + delta;
        self.results_scroll = next.max(0) as u16;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn app() -> App {
        App::new(
            "test".into(),
            vec![
                (
                    "CPU".into(),
                    vec![
                        ("int".into(), "Mops/s".into()),
                        ("float".into(), "MFLOP/s".into()),
                    ],
                ),
                ("Memory".into(), vec![("bw".into(), "GiB/s".into())]),
            ],
            Arc::new(AtomicBool::new(false)),
        )
    }

    #[test]
    fn fraction_advances_with_events() {
        let mut a = app();
        assert_eq!(a.progress_fraction(), 0.0);

        a.apply(Msg::Progress(Ev::SubtestStart {
            bench: 0,
            sub: 0,
            timed: 4,
        }));
        a.apply(Msg::Progress(Ev::Run {
            bench: 0,
            sub: 0,
            run: 2,
            value: 10.0,
        }));
        // 2/4 of the first of three subtests
        assert!((a.progress_fraction() - (0.5 / 3.0)).abs() < 1e-9);

        a.apply(Msg::Progress(Ev::SubtestDone {
            bench: 0,
            sub: 0,
            outcome: Box::new(dummy_outcome()),
        }));
        assert!((a.progress_fraction() - (1.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn active_line_skips_completed_rows() {
        let mut a = app();
        // header(0) sub(1) sub(2) header(3) sub(4)
        assert_eq!(a.active_line(), 1);
        a.apply(Msg::Progress(Ev::SubtestDone {
            bench: 0,
            sub: 0,
            outcome: Box::new(dummy_outcome()),
        }));
        assert_eq!(a.active_line(), 2);
    }

    #[test]
    fn terminal_messages_set_phase() {
        let mut a = app();
        assert!(!a.is_finished());
        a.apply(Msg::Failed("boom".into()));
        assert!(a.is_finished());
    }

    fn dummy_outcome() -> SubtestOutcome {
        use crate::engine::Direction;
        use crate::engine::stats::Stats;
        SubtestOutcome {
            id: "int".into(),
            label: "int".into(),
            unit: "Mops/s".into(),
            direction: Direction::HigherIsBetter,
            value: 10.0,
            stats: Stats::from_runs(vec![10.0]),
            confidence: crate::engine::stats::Confidence::High,
        }
    }
}

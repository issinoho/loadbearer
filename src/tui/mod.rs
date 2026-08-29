//! Interactive run screen. The benchmark engine runs on a worker thread and
//! streams progress to this UI loop over a channel; the loop owns the terminal,
//! draws frames, and handles input.

mod app;
mod compare;
mod view;

pub use compare::run as run_compare;

use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Result, bail};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use app::{App, Ev, Msg, Phase};

use crate::engine::{Benchmark, Progress, ProgressEvent, RunContext, run_benchmark};
use crate::inventory::Inventory;
use crate::scoring::{Baseline, Profile, ResultFile, RunConfig, score_run};

/// Everything the worker thread needs to run and score a session.
pub struct RunInit {
    pub header: String,
    pub selected: Vec<Box<dyn Benchmark>>,
    pub ctx: RunContext,
    pub baseline: Baseline,
    pub profile: Profile,
    pub curve_k: f64,
    pub machine: Inventory,
    pub config: RunConfig,
    /// When set, a sustained-load phase runs after scoring and streams samples.
    pub soak: Option<crate::soak::SoakConfig>,
}

/// Run the interactive session. `Ok(Some(result))` on completion, `Ok(None)` if
/// the user cancelled, `Err` if the run itself failed.
pub fn run(init: RunInit) -> Result<Option<ResultFile>> {
    let abort = init.ctx.abort.clone();
    let header = init.header.clone();
    let specs: Vec<(String, Vec<(String, String)>)> = init
        .selected
        .iter()
        .map(|b| {
            (
                b.label().to_string(),
                b.subtests()
                    .iter()
                    .map(|s| (s.label.to_string(), s.unit.to_string()))
                    .collect(),
            )
        })
        .collect();

    let (tx, rx) = mpsc::channel();
    let worker = spawn_worker(init, tx);

    let mut terminal = ratatui::init();
    let mut app = App::new(header, specs, abort);
    let result = event_loop(&mut terminal, &mut app, &rx);
    ratatui::restore();

    // The worker either finished on its own or noticed the abort flag.
    let _ = worker.join();
    result
}

fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    rx: &Receiver<Msg>,
) -> Result<Option<ResultFile>> {
    loop {
        terminal.draw(|f| view::draw(f, &mut *app))?;

        while let Ok(msg) = rx.try_recv() {
            app.apply(msg);
        }
        if app.cancelling && app.is_finished() {
            return resolve(app);
        }

        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            let ctrl_c =
                key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    if app.is_finished() {
                        return resolve(app);
                    }
                    if !app.cancelling {
                        log::info!(target: "loadbearer::tui", "cancel requested (q/Esc) — finishing current measurement");
                    }
                    app.cancelling = true;
                    app.abort.store(true, Ordering::Relaxed);
                }
                KeyCode::Enter if app.is_finished() => return resolve(app),
                KeyCode::Up | KeyCode::Char('k') => app.scroll_results(-1),
                KeyCode::Down | KeyCode::Char('j') => app.scroll_results(1),
                KeyCode::PageUp => app.scroll_results(-10),
                KeyCode::PageDown | KeyCode::Char(' ') => app.scroll_results(10),
                KeyCode::Home | KeyCode::Char('g') if app.is_finished() => app.results_scroll = 0,
                KeyCode::End | KeyCode::Char('G') if app.is_finished() => {
                    app.results_scroll = u16::MAX
                }
                _ if ctrl_c => {
                    if !app.cancelling {
                        log::info!(target: "loadbearer::tui", "cancel requested (Ctrl-C) — finishing current measurement");
                    }
                    app.cancelling = true;
                    app.abort.store(true, Ordering::Relaxed);
                }
                _ => {}
            }
        }
    }
}

fn resolve(app: &mut App) -> Result<Option<ResultFile>> {
    let aborted = app.abort.load(Ordering::Relaxed);
    match std::mem::replace(&mut app.phase, Phase::Running) {
        Phase::Done(result) => Ok(Some(*result)),
        Phase::Failed(_) if aborted => Ok(None),
        Phase::Failed(err) => bail!("{err}"),
        Phase::Running => Ok(None),
    }
}

fn spawn_worker(init: RunInit, tx: Sender<Msg>) -> JoinHandle<()> {
    thread::spawn(move || {
        let RunInit {
            selected,
            ctx,
            baseline,
            profile,
            curve_k,
            machine,
            config,
            soak,
            ..
        } = init;

        log::debug!(target: "loadbearer::tui", "worker thread started");
        let mut progress = ChannelProgress::new(tx.clone());
        let mut outcomes = Vec::with_capacity(selected.len());
        for bench in &selected {
            match run_benchmark(bench.as_ref(), &ctx, &mut progress) {
                Ok(outcome) => outcomes.push(outcome),
                Err(err) => {
                    log::warn!(target: "loadbearer::tui", "worker: benchmark {} failed: {err:#}", bench.id());
                    let _ = tx.send(Msg::Failed(format!("{err:#}")));
                    return;
                }
            }
        }

        let scored = match score_run(&outcomes, &baseline, profile, curve_k) {
            Ok(scored) => scored,
            Err(err) => {
                let _ = tx.send(Msg::Failed(format!("{err:#}")));
                return;
            }
        };
        let mut result = ResultFile::assemble(machine, config, outcomes, scored, None);

        // Sustained-load phase: stream a sample per second to the UI, then
        // fold the analysed result into the result file. A cancel during the
        // soak still keeps the completed grade — the partial soak is dropped.
        let aborted = || ctx.abort.load(Ordering::Relaxed);
        if let Some(cfg) = soak.filter(|_| !aborted()) {
            let _ = tx.send(Msg::Progress(Ev::SoakStart {
                duration_secs: cfg.duration.as_secs_f64(),
                threads: cfg.threads,
            }));
            let tx_soak = tx.clone();
            let sr = crate::soak::run(&cfg, ctx.abort.as_ref(), move |s| {
                let _ = tx_soak.send(Msg::Progress(Ev::SoakSample { sample: *s }));
            });
            // Keep whatever ran (the user watched it live); only drop it if the
            // soak was cancelled before it produced a usable series.
            if sr.samples.len() >= 3 {
                result.soak = Some(sr);
            }
        }

        let _ = tx.send(Msg::Done(Box::new(result)));
    })
}

/// Converts the engine's borrowed [`ProgressEvent`]s into owned [`Msg`]s. The
/// engine visits benchmarks and subtests strictly in order, so a running cursor
/// is enough to attach indices.
struct ChannelProgress {
    tx: Sender<Msg>,
    bench: Option<usize>,
    sub: usize,
}

impl ChannelProgress {
    fn new(tx: Sender<Msg>) -> Self {
        Self {
            tx,
            bench: None,
            sub: 0,
        }
    }

    fn send(&self, ev: Ev) {
        let _ = self.tx.send(Msg::Progress(ev));
    }
}

impl Progress for ChannelProgress {
    fn on_event(&mut self, event: ProgressEvent<'_>) {
        match event {
            ProgressEvent::BenchStart { .. } => {
                self.bench = Some(self.bench.map_or(0, |b| b + 1));
                self.sub = 0;
                self.send(Ev::BenchStart);
            }
            ProgressEvent::SubtestStart { timed_runs, .. } => self.send(Ev::SubtestStart {
                bench: self.bench.unwrap_or(0),
                sub: self.sub,
                timed: timed_runs,
            }),
            ProgressEvent::Warmup { .. } => self.send(Ev::Warmup {
                bench: self.bench.unwrap_or(0),
                sub: self.sub,
            }),
            ProgressEvent::Run { run, value, .. } => self.send(Ev::Run {
                bench: self.bench.unwrap_or(0),
                sub: self.sub,
                run,
                value,
            }),
            ProgressEvent::SubtestDone { outcome, .. } => {
                self.send(Ev::SubtestDone {
                    bench: self.bench.unwrap_or(0),
                    sub: self.sub,
                    outcome: Box::new(outcome.clone()),
                });
                self.sub += 1;
            }
            ProgressEvent::BenchDone { .. } => self.send(Ev::BenchDone {
                bench: self.bench.unwrap_or(0),
            }),
        }
    }
}

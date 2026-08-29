//! All ratatui drawing for the run screen and the results screen.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Wrap};

use super::app::{App, Phase, SoakView, SubRow, SubState};
use crate::scoring::{Grade, ResultFile};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    if area.width < 44 || area.height < 12 {
        f.render_widget(
            Paragraph::new("Terminal too small — enlarge the window.").alignment(Alignment::Center),
            area,
        );
        return;
    }
    match &app.phase {
        Phase::Running => match &app.soak {
            Some(soak) => draw_soak(f, &app.header, soak, app.cancelling, area),
            None => draw_running(f, app, area),
        },
        Phase::Failed(err) => draw_failed(f, err, area),
        // `results_scroll` is a disjoint field from `phase`, so this mutable
        // borrow alongside the shared borrow of `phase` is fine.
        Phase::Done(result) => draw_results(f, &mut app.results_scroll, result, area),
    }
}

fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

fn bar(frac: f64, width: usize) -> String {
    let f = frac.clamp(0.0, 1.0);
    let filled = (f * width as f64).round() as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

fn fmt_val(v: f64) -> String {
    if v >= 1000.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
}

fn grade_color(g: Grade) -> Color {
    match g {
        Grade::S | Grade::A => Color::Green,
        Grade::B => ACCENT,
        Grade::C => Color::Yellow,
        Grade::D | Grade::F => Color::Red,
    }
}

// --- running screen ---------------------------------------------------------

fn draw_running(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(" loadbearer — running ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // spacer
        Constraint::Min(3),    // list
        Constraint::Length(1), // gauge
        Constraint::Length(1), // hint
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            &app.header,
            Style::default().fg(ACCENT),
        )])),
        rows[0],
    );

    // Label column: what's left after the fixed bar / status / value / conf
    // columns and their gaps, within bounds.
    let name_w = (inner.width as usize).saturating_sub(48).clamp(22, 46);

    let mut lines: Vec<Line> = Vec::new();
    for b in &app.benches {
        let tick = if b.done { " ✓" } else { "" };
        lines.push(Line::from(Span::styled(
            format!("{}{}", b.label, tick),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for s in &b.subs {
            lines.push(subtest_line(s, name_w));
        }
    }

    let viewport = rows[2].height.max(1) as usize;
    let scroll = app
        .active_line()
        .saturating_sub(viewport.saturating_sub(2))
        .min(lines.len().saturating_sub(viewport)) as u16;
    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), rows[2]);

    let frac = app.progress_fraction();
    let elapsed = app.started.elapsed();
    let eta = if frac > 0.05 && frac < 1.0 {
        let total = elapsed.as_secs_f64() / frac;
        format!(
            " · eta {}",
            fmt_dur(Duration::from_secs_f64(total - elapsed.as_secs_f64()))
        )
    } else {
        String::new()
    };
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(ACCENT))
            .ratio(frac.clamp(0.0, 1.0))
            .label(format!(
                "{:.0}% · elapsed {}{}",
                frac * 100.0,
                fmt_dur(elapsed),
                eta
            )),
        rows[3],
    );

    let hint = if app.cancelling {
        "cancelling — finishing the current measurement…"
    } else {
        "q  quit"
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(DIM))),
        rows[4],
    );
}

fn subtest_line(s: &SubRow, name_w: usize) -> Line<'static> {
    // 7-wide status field so the columns after it stay aligned across states.
    let (status, style) = match s.state {
        SubState::Pending => ("       ".to_string(), Style::default().fg(DIM)),
        SubState::Warmup => ("warmup ".to_string(), Style::default().fg(Color::Yellow)),
        SubState::Running => (
            format!("{:>3}/{:<3}", s.runs_done, s.timed),
            Style::default().fg(Color::Yellow),
        ),
        SubState::Done => ("done   ".to_string(), Style::default().fg(Color::Green)),
    };

    // Number right-aligned, unit left-aligned, so both columns line up down
    // the list regardless of magnitude or unit length.
    let value = match s.display_value() {
        Some(v) => format!("{:>7} {:<7}", fmt_val(v), s.unit),
        None => String::new(),
    };
    let conf = match (&s.state, &s.outcome) {
        (SubState::Done, Some(o)) => o.confidence.as_str(),
        _ => "",
    };

    Line::from(vec![
        Span::raw(format!("  {:<name_w$}  ", truncate(&s.label, name_w))),
        Span::styled(bar(s.fraction(), 10), style),
        Span::styled(format!("  {status}  "), style),
        Span::raw(format!("{value:<15}")),
        Span::styled(format!("  {conf}"), Style::default().fg(DIM)),
    ])
}

// --- soak screen ----------------------------------------------------------

fn draw_soak(f: &mut Frame, header: &str, soak: &SoakView, cancelling: bool, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(" loadbearer — soak (sustained load) ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // spacer
        Constraint::Length(1), // subtitle
        Constraint::Length(1), // spacer
        Constraint::Min(6),    // body
        Constraint::Length(1), // gauge
        Constraint::Length(1), // hint
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(Span::styled(
            header.to_string(),
            Style::default().fg(ACCENT),
        )),
        rows[0],
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            format!(
                "sustained all-core load · {} threads · not scored",
                soak.threads
            ),
            Style::default().fg(DIM),
        )),
        rows[2],
    );

    let peak = soak.peak_so_far();
    let latest = soak.latest_rate().unwrap_or(0.0);
    let mut body: Vec<Line> = Vec::new();
    body.push(kv_line(
        "throughput",
        format!("{:>10.0} Mops/s", latest),
        if peak > 0.0 {
            format!("   peak so far {peak:.0}")
        } else {
            String::new()
        },
    ));
    body.push(kv_line(
        "clock",
        match soak.latest_mhz() {
            Some(m) => format!("{:>10.2} GHz", m as f64 / 1000.0),
            None => "         —".to_string(),
        },
        String::new(),
    ));
    body.push(kv_line(
        "retained",
        match soak.retained_so_far() {
            Some(p) => format!("{p:>9.0}%"),
            None => "         —".to_string(),
        },
        "   current vs peak so far".to_string(),
    ));
    body.push(Line::raw(""));
    let spark = soak_spark(soak, inner.width.saturating_sub(6).min(72) as usize);
    if !spark.is_empty() {
        body.push(Line::from(Span::styled(
            format!("  {spark}"),
            Style::default().fg(ACCENT),
        )));
        body.push(Line::from(Span::styled(
            "  throughput, one mark per sample",
            Style::default().fg(DIM),
        )));
    }
    f.render_widget(Paragraph::new(body), rows[4]);

    let frac = soak.elapsed_frac();
    let remaining = (soak.duration_secs * (1.0 - frac)).max(0.0);
    f.render_widget(
        Gauge::default()
            .gauge_style(Style::default().fg(ACCENT))
            .ratio(frac)
            .label(format!(
                "{:.0}% · {}s / {:.0}s",
                frac * 100.0,
                soak.started.elapsed().as_secs(),
                soak.duration_secs,
            )),
        rows[5],
    );

    let hint = if cancelling {
        "cancelling — keeping the graded result…".to_string()
    } else {
        format!("q  skip the soak (keeps the graded result) · ~{remaining:.0}s left")
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(DIM))),
        rows[6],
    );
}

fn kv_line(key: &str, value: String, tail: String) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("  {key:<12} ")),
        Span::styled(value, Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(tail, Style::default().fg(DIM)),
    ])
}

/// A Unicode sparkline of the soak samples so far, tail-trimmed to `width`.
fn soak_spark(soak: &SoakView, width: usize) -> String {
    const TICKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if soak.samples.is_empty() || width == 0 {
        return String::new();
    }
    let rates: Vec<f64> = soak.samples.iter().map(|s| s.rate).collect();
    let tail = &rates[rates.len().saturating_sub(width)..];
    let lo = tail.iter().copied().fold(f64::MAX, f64::min);
    let hi = tail.iter().copied().fold(f64::MIN, f64::max);
    let span = (hi - lo).max(1e-9);
    tail.iter()
        .map(|&v| {
            let idx = (((v - lo) / span) * (TICKS.len() - 1) as f64).round() as usize;
            TICKS[idx.min(TICKS.len() - 1)]
        })
        .collect()
}

// --- results screen --------------------------------------------------------

fn draw_results(f: &mut Frame, scroll: &mut u16, r: &ResultFile, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(grade_color(r.overall.grade)))
        .title(" loadbearer — assessment ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(inner);

    // Subtest label column: whatever is left after the fixed value / unit /
    // ratio / score / confidence columns, within bounds.
    let name_w = (inner.width as usize).saturating_sub(46).clamp(22, 40);

    let mut lines: Vec<Line> = Vec::new();
    let m = &r.machine;
    lines.push(Line::from(Span::styled(
        format!(
            "{} · {} · {} threads · {} RAM",
            m.hostname.as_deref().unwrap_or("unknown"),
            m.cpu_model,
            m.cpu_logical_cores,
            crate::output::human_bytes(m.ram_bytes),
        ),
        Style::default().fg(ACCENT),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            "{} · k={} · baseline {} · {} preset",
            r.config.profile, r.config.curve_k, r.config.baseline, r.config.duration_preset
        ),
        Style::default().fg(DIM),
    )));
    lines.push(Line::raw(""));

    for c in &r.components {
        lines.push(component_line(&c.label, c.score, c.grade, c.graded));
        for st in &c.subtests {
            lines.push(Line::from(Span::styled(
                format!(
                    "    {:<name_w$} {:>11} {:<7} {:>5.2}x  {:>5.0}  {}",
                    truncate(&st.label, name_w),
                    fmt_val(st.value),
                    st.unit,
                    st.ratio,
                    st.score,
                    st.confidence.as_str(),
                ),
                Style::default().fg(DIM),
            )));
        }
        for note in &c.notes {
            lines.push(Line::from(Span::styled(
                format!("    note: {note}"),
                Style::default().fg(Color::Yellow),
            )));
        }
        lines.push(Line::raw(""));
    }

    if r.components.iter().any(|c| c.graded) {
        lines.push(component_line(
            "OVERALL",
            r.overall.score,
            r.overall.grade,
            true,
        ));
    } else {
        lines.push(Line::from(Span::styled(
            "  OVERALL    —  no graded components in this run".to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Why:",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for w in &r.overall.why {
        lines.push(Line::raw(format!("  - {w}")));
    }
    let ungraded: Vec<&str> = r
        .components
        .iter()
        .filter(|c| !c.graded)
        .map(|c| c.label.as_str())
        .collect();
    if !ungraded.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {} — shown, not in the overall", ungraded.join(" and ")),
            Style::default().fg(DIM),
        )));
    }

    if let Some(b) = &r.machine.battery {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            format!("BATTERY  ({} · not graded)", b.summary()),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            format!("  charge {:.0}% ({})", b.charge_pct, b.state),
            Style::default().fg(DIM),
        )));
        let health = match (b.energy_full_wh, b.energy_full_design_wh, b.health_pct) {
            (Some(full), Some(design), Some(h)) => {
                format!("  health {full:.1} / {design:.1} Wh design ({h:.0}%)")
            }
            (_, _, Some(h)) => format!("  health {h:.0}% of design capacity"),
            _ => "  health unavailable (no design-capacity reading)".to_string(),
        };
        lines.push(Line::from(Span::styled(health, Style::default().fg(DIM))));
        if let Some(v) = b.health_verdict() {
            lines.push(Line::from(Span::styled(
                format!("  → {v}"),
                Style::default().fg(DIM),
            )));
        }
        if let Some(n) = b.power_note() {
            lines.push(Line::from(Span::styled(
                format!("  note: {n}"),
                Style::default().fg(Color::Yellow),
            )));
        }
    }

    if let Some(s) = &r.soak {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            format!("SOAK  ({:.0}s sustained · not graded)", s.duration_secs),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::raw(format!(
            "  peak {:.0} · steady {:.0} {} · {:.0}% retained",
            s.peak_rate, s.steady_rate, s.unit, s.retained_pct,
        )));
        let onset = match s.onset_secs {
            Some(t) => format!("throttles from ~{t:.0}s"),
            None => "no clear throttle onset".to_string(),
        };
        let clock = if s.mhz_peak > 0 {
            format!(
                " · clock {:.2}→{:.2} GHz",
                s.mhz_peak as f64 / 1000.0,
                s.mhz_steady as f64 / 1000.0,
            )
        } else {
            String::new()
        };
        lines.push(Line::from(Span::styled(
            format!("  {onset}{clock}"),
            Style::default().fg(DIM),
        )));
    }

    let viewport = rows[0].height as usize;
    let max_scroll = lines.len().saturating_sub(viewport) as u16;
    *scroll = (*scroll).min(max_scroll);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((*scroll, 0)),
        rows[0],
    );

    let hint = if max_scroll > 0 {
        format!("↑/↓ scroll · {}/{} · q / enter  exit", *scroll, max_scroll)
    } else {
        "q / enter  exit".to_string()
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(DIM))),
        rows[1],
    );
}

fn component_line(label: &str, score: f64, grade: Grade, graded: bool) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            format!("  {:<9}", label.to_uppercase()),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("{score:>5.0}  ")),
        Span::styled(
            format!("[{}]", grade.as_str()),
            Style::default()
                .fg(grade_color(grade))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            bar(score / 1500.0, 22),
            Style::default().fg(grade_color(grade)),
        ),
    ];
    if !graded {
        spans.push(Span::styled("  · not in overall", Style::default().fg(DIM)));
    }
    Line::from(spans)
}

fn draw_failed(f: &mut Frame, err: &str, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(" loadbearer — failed ");
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(
        Paragraph::new(vec![
            Line::raw(""),
            Line::from(Span::styled(
                format!("  {err}"),
                Style::default().fg(Color::Red),
            )),
            Line::raw(""),
            Line::from(Span::styled("  q  quit", Style::default().fg(DIM))),
        ])
        .wrap(Wrap { trim: false }),
        inner,
    );
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let keep: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{keep}…")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::engine::stats::{Confidence, Stats};
    use crate::engine::{Direction, SubtestOutcome};
    use crate::tui::app::{App, Ev, Msg};

    /// Render `draw_running` at `width` with the given `(label, unit)` subtests,
    /// each marked done with `value`. Returns the screen as one row per line.
    fn render(width: u16, subs: &[(&str, &str, f64)]) -> Vec<String> {
        let specs = vec![(
            "CPU".to_string(),
            subs.iter()
                .map(|(l, u, _)| (l.to_string(), u.to_string()))
                .collect(),
        )];
        let mut app = App::new("host".into(), specs, Arc::new(AtomicBool::new(false)));
        for (i, (label, unit, value)) in subs.iter().enumerate() {
            app.apply(Msg::Progress(Ev::SubtestDone {
                bench: 0,
                sub: i,
                outcome: Box::new(SubtestOutcome {
                    id: (*label).into(),
                    label: (*label).into(),
                    unit: (*unit).into(),
                    direction: Direction::HigherIsBetter,
                    value: *value,
                    stats: Stats::from_runs(vec![*value]),
                    confidence: Confidence::Medium,
                }),
            }));
        }
        let mut term = Terminal::new(TestBackend::new(width, 20)).unwrap();
        term.draw(|f| draw_running(f, &app, f.area())).unwrap();
        let buf = term.backend().buffer().clone();
        let a = buf.area();
        (0..a.height)
            .map(|y| (0..a.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn wide_terminal_shows_long_labels_in_full() {
        let rows = render(
            110,
            &[
                ("TCP throughput, single stream", "GiB/s", 3.01),
                ("Sequential read, all cores", "GiB/s", 12.6),
            ],
        );
        let screen = rows.join("\n");
        assert!(screen.contains("TCP throughput, single stream"), "{screen}");
        assert!(screen.contains("Sequential read, all cores"), "{screen}");
        assert!(!screen.contains('…'));
    }

    #[test]
    fn narrow_terminal_truncates_with_an_ellipsis() {
        let rows = render(60, &[("TCP throughput, single stream", "GiB/s", 3.01)]);
        let screen = rows.join("\n");
        assert!(screen.contains('…'), "{screen}");
        assert!(!screen.contains("TCP throughput, single stream"));
    }

    #[test]
    fn value_and_confidence_columns_align_regardless_of_label_or_magnitude() {
        let rows = render(
            100,
            &[
                ("SHA-256 hash", "MiB/s", 130.1),
                ("Sequential read, all cores", "GiB/s", 9.4),
            ],
        );
        let a = rows.iter().find(|r| r.contains("SHA-256 hash")).unwrap();
        let b = rows
            .iter()
            .find(|r| r.contains("Sequential read, all cores"))
            .unwrap();
        assert_eq!(a.find("done"), b.find("done"), "status column\n{a}\n{b}");
        assert_eq!(
            a.rfind("medium"),
            b.rfind("medium"),
            "confidence column\n{a}\n{b}"
        );
    }
}

//! All ratatui drawing for the run screen and the results screen.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Wrap};

use super::app::{App, Phase, SubRow, SubState};
use crate::scoring::{Grade, ResultFile};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < 44 || area.height < 12 {
        f.render_widget(
            Paragraph::new("Terminal too small — enlarge the window.").alignment(Alignment::Center),
            area,
        );
        return;
    }
    match &app.phase {
        Phase::Running => draw_running(f, app, area),
        Phase::Done(result) => draw_results(f, app, result, area),
        Phase::Failed(err) => draw_failed(f, err, area),
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

    let mut lines: Vec<Line> = Vec::new();
    for b in &app.benches {
        let tick = if b.done { " ✓" } else { "" };
        lines.push(Line::from(Span::styled(
            format!("{}{}", b.label, tick),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for s in &b.subs {
            lines.push(subtest_line(s));
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

fn subtest_line(s: &SubRow) -> Line<'static> {
    let (status, style) = match s.state {
        SubState::Pending => ("        ".to_string(), Style::default().fg(DIM)),
        SubState::Warmup => ("warmup  ".to_string(), Style::default().fg(Color::Yellow)),
        SubState::Running => (
            format!("{:>3}/{:<3}", s.runs_done, s.timed),
            Style::default().fg(Color::Yellow),
        ),
        SubState::Done => ("done    ".to_string(), Style::default().fg(Color::Green)),
    };

    let value = match s.display_value() {
        Some(v) => format!("{:>10} {}", fmt_val(v), s.unit),
        None => String::new(),
    };
    let conf = match (&s.state, &s.outcome) {
        (SubState::Done, Some(o)) => format!("  {}", o.confidence.as_str()),
        _ => String::new(),
    };

    Line::from(vec![
        Span::raw(format!("  {:<22} ", truncate(&s.label, 22))),
        Span::styled(bar(s.fraction(), 10), style),
        Span::styled(format!(" {status} "), style),
        Span::raw(value),
        Span::styled(conf, Style::default().fg(DIM)),
    ])
}

// --- results screen --------------------------------------------------------

fn draw_results(f: &mut Frame, app: &App, r: &ResultFile, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(grade_color(r.overall.grade)))
        .title(" loadbearer — assessment ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(inner);

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
                    "    {:<24} {:>11} {:<7} {:>5.2}x  {:>5.0}  {}",
                    truncate(&st.label, 24),
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

    lines.push(component_line(
        "OVERALL",
        r.overall.score,
        r.overall.grade,
        true,
    ));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Why:",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for w in &r.overall.why {
        lines.push(Line::raw(format!("  - {w}")));
    }
    if r.components.iter().any(|c| !c.graded) {
        lines.push(Line::from(Span::styled(
            "  network reflects the OS network stack — shown, not in the overall",
            Style::default().fg(DIM),
        )));
    }

    let viewport = rows[0].height as usize;
    let max_scroll = lines.len().saturating_sub(viewport) as u16;
    let scroll = app.results_scroll.min(max_scroll);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        rows[0],
    );

    let hint = if max_scroll > 0 {
        "↑/↓ scroll · q / enter  exit"
    } else {
        "q / enter  exit"
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

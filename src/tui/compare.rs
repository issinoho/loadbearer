//! Scrollable terminal view for `loadbearer compare`. Unlike the run screen
//! there is no worker thread — the [`Comparison`] is already computed, so this
//! is a static, coloured, scrollable rendering of the same table
//! [`crate::output::print_comparison`] prints.

use std::time::Duration;

use anyhow::Result;
use ratatui::Frame;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::compare::Comparison;

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const NAME_W: usize = 30;
const COL_W: usize = 15;

/// Show the comparison in an alternate screen until the user quits.
pub fn run(c: &Comparison) -> Result<()> {
    let lines = build_lines(c);

    let mut terminal = ratatui::init();
    let mut scroll: u16 = 0;
    let outcome = loop {
        if let Err(e) = terminal.draw(|f| draw(f, &lines, &mut scroll)) {
            break Err(e.into());
        }
        match event::poll(Duration::from_millis(200)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => break Err(e.into()),
        }
        let Ok(Event::Key(key)) = event::read() else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl_c =
            key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter => break Ok(()),
            _ if ctrl_c => break Ok(()),
            KeyCode::Up | KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => scroll = scroll.saturating_add(1),
            KeyCode::PageUp => scroll = scroll.saturating_sub(10),
            KeyCode::PageDown | KeyCode::Char(' ') => scroll = scroll.saturating_add(10),
            KeyCode::Home | KeyCode::Char('g') => scroll = 0,
            KeyCode::End | KeyCode::Char('G') => scroll = u16::MAX,
            _ => {}
        }
    };
    ratatui::restore();
    outcome
}

fn draw(f: &mut Frame, lines: &[Line<'static>], scroll: &mut u16) {
    let area = f.area();
    if area.width < 40 || area.height < 8 {
        f.render_widget(
            Paragraph::new("Terminal too small — enlarge the window.").centered(),
            area,
        );
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
        .title(" loadbearer — compare ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(inner);

    let viewport = rows[0].height as usize;
    let max_scroll = lines.len().saturating_sub(viewport) as u16;
    *scroll = (*scroll).min(max_scroll);

    f.render_widget(Paragraph::new(lines.to_vec()).scroll((*scroll, 0)), rows[0]);

    let hint = if max_scroll > 0 {
        format!("↑/↓ scroll · {}/{} · q exit", *scroll, max_scroll)
    } else {
        "q  exit".to_string()
    };
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(DIM))),
        rows[1],
    );
}

// --- line building --------------------------------------------------------

fn build_lines(c: &Comparison) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let n = c.machines.len();

    out.push(Line::from(Span::styled(
        format!("{n} machines"),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for m in &c.machines {
        out.push(Line::from(vec![
            Span::styled(
                format!("  {}  ", m.tag),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("{:<22} {}", truncate(&m.label, 22), m.cpu_model)),
        ]));
        out.push(Line::from(Span::styled(
            format!(
                "     {} preset · baseline {} · k={} · stored {:.0} [{}]",
                m.duration_preset,
                m.baseline,
                m.curve_k,
                m.stored_overall_score,
                m.stored_overall_grade,
            ),
            Style::default().fg(DIM),
        )));
    }

    if !c.warnings.is_empty() {
        out.push(Line::raw(""));
        for w in &c.warnings {
            out.push(Line::from(Span::styled(
                format!("  ! {w}"),
                Style::default().fg(Color::Yellow),
            )));
        }
    }

    out.push(Line::raw(""));
    let mut hdr = vec![Span::raw(format!("  {:<NAME_W$}", "metric"))];
    for m in &c.machines {
        hdr.push(Span::styled(
            format!(
                " {:>COL_W$}",
                truncate(&format!("{}: {}", m.tag, m.label), COL_W)
            ),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }
    out.push(Line::from(hdr));

    for comp in &c.components {
        out.push(Line::raw(""));
        out.push(Line::from(Span::styled(
            format!("  {}", comp.label.to_uppercase()),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )));
        for st in &comp.subtests {
            out.push(metric_row(
                c,
                &format!("{} ({})", st.label, st.unit),
                &st.values,
                &st.rel,
                st.best,
                false,
            ));
        }
        out.push(metric_row(
            c,
            "  → component",
            &vec![f64::NAN; n],
            &comp.rel,
            comp.best,
            true,
        ));
    }

    out.push(Line::raw(""));
    out.push(metric_row(
        c,
        "OVERALL",
        &vec![f64::NAN; n],
        &c.overall.rel,
        c.overall.ranking[0],
        true,
    ));

    if let Some(sk) = &c.soak {
        out.push(Line::raw(""));
        out.push(Line::from(Span::styled(
            "  SUSTAINED LOAD (not graded)".to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        out.push(metric_row(
            c,
            &format!("  steady ({})", sk.unit),
            &sk.steady_rate,
            &sk.rel_steady,
            sk.best_sustained,
            false,
        ));
        let mut row = vec![Span::raw(format!(
            "  {:<NAME_W$}",
            "  retained vs own peak"
        ))];
        for (i, v) in sk.retained_pct.iter().enumerate() {
            let style = if i == sk.best_retention {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };
            row.push(Span::styled(
                format!(" {:>COL_W$}", format!("{v:.0}%")),
                style,
            ));
        }
        row.push(Span::styled(
            format!("  {}", c.machines[sk.best_retention].tag),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
        out.push(Line::from(row));
    }

    out.push(Line::raw(""));
    out.push(Line::from(vec![
        Span::styled("  Verdict: ", Style::default().add_modifier(Modifier::BOLD)),
        Span::styled(c.overall.summary.clone(), Style::default().fg(ACCENT)),
    ]));

    out
}

/// One metric row: a label, one cell per machine (value + delta, or just the
/// delta for aggregate rows where `values` is all-NaN), and a winner tag.
fn metric_row(
    c: &Comparison,
    label: &str,
    values: &[f64],
    rel: &[f64],
    best: usize,
    aggregate: bool,
) -> Line<'static> {
    let mut spans = vec![Span::raw(format!("  {:<NAME_W$}", truncate(label, NAME_W)))];
    for (i, r) in rel.iter().enumerate() {
        let cell = if i == 0 {
            if aggregate {
                "ref".to_string()
            } else {
                fmt_val(values[i])
            }
        } else {
            let pct = (r - 1.0) * 100.0;
            if aggregate {
                format!("{pct:+.0}%")
            } else {
                format!("{} {pct:+.0}%", fmt_val(values[i]))
            }
        };
        let style = match () {
            _ if i == 0 => Style::default().fg(DIM),
            _ if i == best && rel.len() > 1 => Style::default().fg(Color::Green),
            _ if *r < 0.98 => Style::default().fg(Color::Red),
            _ => Style::default(),
        };
        spans.push(Span::styled(format!(" {cell:>COL_W$}"), style));
    }
    spans.push(winner_span(c, rel, best));
    Line::from(spans)
}

/// The winning machine's tag, or `=` when the top two are within 2 %.
fn winner_span(c: &Comparison, rel: &[f64], best: usize) -> Span<'static> {
    let mut sorted: Vec<f64> = rel.to_vec();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let ambiguous = sorted.len() < 2 || sorted[1] <= 0.0 || sorted[0] / sorted[1] - 1.0 < 0.02;
    if ambiguous {
        Span::styled("  =".to_string(), Style::default().fg(DIM))
    } else {
        Span::styled(
            format!("  {}", c.machines[best].tag),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    }
}

fn fmt_val(v: f64) -> String {
    if v.is_nan() {
        String::new()
    } else if v >= 1000.0 {
        format!("{v:.0}")
    } else if v >= 10.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.2}")
    }
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
    use super::*;
    use crate::compare::{
        ComponentComparison, MachineRef, OverallComparison, SoakComparison, SubtestComparison,
    };
    use crate::engine::Direction;

    fn machine(tag: &str, label: &str) -> MachineRef {
        MachineRef {
            tag: tag.into(),
            label: label.into(),
            path: format!("{label}.json"),
            cpu_model: "Test CPU".into(),
            baseline: "reference-v1".into(),
            curve_k: 0.5,
            duration_preset: "short".into(),
            profile: "general".into(),
            stored_overall_score: 1000.0,
            stored_overall_grade: "B".into(),
        }
    }

    fn two_machine_comparison(soak: bool) -> Comparison {
        let st = SubtestComparison {
            id: "int_single".into(),
            label: "Integer, single-core".into(),
            unit: "Mops/s".into(),
            direction: Direction::HigherIsBetter,
            values: vec![9000.0, 12000.0],
            rel: vec![1.0, 1.333],
            best: 1,
        };
        let comp = ComponentComparison {
            id: "cpu".into(),
            label: "CPU".into(),
            subtests: vec![st],
            rel: vec![1.0, 1.333],
            best: 1,
        };
        Comparison {
            machines: vec![machine("A", "alpha"), machine("B", "bravo")],
            warnings: vec!["files used different duration presets".into()],
            components: vec![comp],
            overall: OverallComparison {
                rel: vec![1.0, 1.333],
                ranking: vec![1, 0],
                summary: "bravo leads by 33% overall (ahead on cpu +33%).".into(),
            },
            soak: soak.then(|| SoakComparison {
                unit: "Mops/s".into(),
                steady_rate: vec![18000.0, 20000.0],
                peak_rate: vec![19000.0, 24000.0],
                retained_pct: vec![94.7, 83.3],
                rel_steady: vec![1.0, 1.111],
                best_retention: 0,
                best_sustained: 1,
            }),
        }
    }

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn build_lines_covers_every_section() {
        let lines = build_lines(&two_machine_comparison(true));
        let joined = lines.iter().map(text).collect::<Vec<_>>().join("\n");

        assert!(joined.contains("2 machines"));
        assert!(joined.contains("A  alpha"));
        assert!(joined.contains("! files used different duration presets"));
        assert!(joined.contains("CPU"));
        assert!(joined.contains("Integer, single-core (Mops/s)"));
        assert!(joined.contains("→ component"));
        assert!(joined.contains("OVERALL"));
        assert!(joined.contains("SUSTAINED LOAD (not graded)"));
        assert!(joined.contains("retained vs own peak"));
        assert!(joined.contains("Verdict: bravo leads by 33% overall"));
    }

    #[test]
    fn winner_and_reference_cells_render() {
        let lines = build_lines(&two_machine_comparison(false));
        let joined = lines.iter().map(text).collect::<Vec<_>>().join("\n");
        // Machine A is the reference; the delta column shows the winner tag B.
        let overall = lines
            .iter()
            .map(text)
            .find(|l| l.trim_start().starts_with("OVERALL"))
            .unwrap();
        assert!(overall.contains("ref"), "{overall:?}");
        assert!(overall.contains("+33%"), "{overall:?}");
        assert!(overall.trim_end().ends_with('B'), "{overall:?}");
        assert!(!joined.contains("SUSTAINED LOAD"));
    }

    #[test]
    fn no_panic_with_the_minimum_two_machines_and_no_warnings_or_soak() {
        let mut c = two_machine_comparison(false);
        c.warnings.clear();
        let _ = build_lines(&c);
    }
}

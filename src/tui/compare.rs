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
const COL_W: usize = 15;
/// Label-column width bounds; the actual width is picked from the terminal.
const NAME_MIN: usize = 22;
const NAME_MAX: usize = 48;

/// Show the comparison in an alternate screen until the user quits.
pub fn run(c: &Comparison) -> Result<()> {
    let mut terminal = ratatui::init();
    let mut scroll: u16 = 0;
    let outcome = loop {
        if let Err(e) = terminal.draw(|f| draw(f, c, &mut scroll)) {
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

fn draw(f: &mut Frame, c: &Comparison, scroll: &mut u16) {
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

    // Give the label column whatever width is left after the value columns and
    // the winner tag, within bounds.
    let n = c.machines.len();
    let name_w = (inner.width as usize)
        .saturating_sub(2 + n * (COL_W + 1) + 4)
        .clamp(NAME_MIN, NAME_MAX);
    let lines = build_lines(c, name_w, inner.width as usize);

    let viewport = rows[0].height as usize;
    let max_scroll = lines.len().saturating_sub(viewport) as u16;
    *scroll = (*scroll).min(max_scroll);

    f.render_widget(Paragraph::new(lines).scroll((*scroll, 0)), rows[0]);

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

fn build_lines(c: &Comparison, name_w: usize, wrap_w: usize) -> Vec<Line<'static>> {
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
            for (i, seg) in wrap_words(w, wrap_w.saturating_sub(4))
                .into_iter()
                .enumerate()
            {
                let prefix = if i == 0 { "  ! " } else { "    " };
                out.push(Line::from(Span::styled(
                    format!("{prefix}{seg}"),
                    Style::default().fg(Color::Yellow),
                )));
            }
        }
    }

    out.push(Line::raw(""));
    let mut hdr = vec![Span::raw(format!("  {:<name_w$}", "metric"))];
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
                name_w,
            ));
        }
        out.push(metric_row(
            c,
            "  → component",
            &vec![f64::NAN; n],
            &comp.rel,
            comp.best,
            true,
            name_w,
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
        name_w,
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
            name_w,
        ));
        let mut row = vec![Span::raw(format!(
            "  {:<name_w$}",
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
    let verdict = wrap_words(&c.overall.summary, wrap_w.saturating_sub(11));
    for (i, seg) in verdict.into_iter().enumerate() {
        if i == 0 {
            out.push(Line::from(vec![
                Span::styled("  Verdict: ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(seg, Style::default().fg(ACCENT)),
            ]));
        } else {
            out.push(Line::from(Span::styled(
                format!("           {seg}"),
                Style::default().fg(ACCENT),
            )));
        }
    }

    out
}

/// Greedy word-wrap to `width` columns. A single word longer than `width` is
/// hard-split into `width`-sized chunks rather than left to overflow.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for raw in text.split_whitespace() {
        let mut chunks: Vec<String> = Vec::new();
        if raw.chars().count() > width {
            let mut buf = String::new();
            for ch in raw.chars() {
                buf.push(ch);
                if buf.chars().count() == width {
                    chunks.push(std::mem::take(&mut buf));
                }
            }
            if !buf.is_empty() {
                chunks.push(buf);
            }
        } else {
            chunks.push(raw.to_string());
        }
        for word in chunks {
            if line.is_empty() {
                line = word;
            } else if line.chars().count() + 1 + word.chars().count() <= width {
                line.push(' ');
                line.push_str(&word);
            } else {
                out.push(std::mem::take(&mut line));
                line = word;
            }
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// One metric row: a label, one cell per machine (value + delta, or just the
/// delta for aggregate rows where `values` is all-NaN), and a winner tag.
#[allow(clippy::too_many_arguments)]
fn metric_row(
    c: &Comparison,
    label: &str,
    values: &[f64],
    rel: &[f64],
    best: usize,
    aggregate: bool,
    name_w: usize,
) -> Line<'static> {
    let mut spans = vec![Span::raw(format!("  {:<name_w$}", truncate(label, name_w)))];
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
        let winner = rel.len() > 1 && i == best;
        let style = if winner {
            Style::default().fg(Color::Green)
        } else if i == 0 {
            Style::default().fg(DIM)
        } else if *r < 0.98 {
            Style::default().fg(Color::Red)
        } else {
            Style::default()
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
        let lines = build_lines(&two_machine_comparison(true), 30, 100);
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
        let lines = build_lines(&two_machine_comparison(false), 30, 100);
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
        let _ = build_lines(&c, 30, 100);
    }

    #[test]
    fn a_long_warning_wraps_onto_indented_continuation_lines() {
        let mut c = two_machine_comparison(false);
        c.warnings = vec![
            "files are from different operating systems (Linux (Ubuntu 24.04), \
             Linux (Ubuntu 26.04)); the network component in particular is not \
             comparable across OSes"
                .to_string(),
        ];
        let lines: Vec<String> = build_lines(&c, 30, 60).iter().map(text).collect();
        let start = lines.iter().position(|l| l.starts_with("  ! ")).unwrap();
        let warn: Vec<&String> = lines[start..]
            .iter()
            .take_while(|l| l.starts_with("  ! ") || l.starts_with("    "))
            .collect();
        assert!(warn.len() > 1, "expected the warning to wrap: {lines:#?}");
        assert!(warn[0].starts_with("  ! "));
        assert!(warn[1..].iter().all(|l| l.starts_with("    ")));
        // Every wrapped warning line fits the 60-col pane.
        assert!(
            warn.iter().all(|l| l.chars().count() <= 60),
            "a warning line overflowed: {warn:#?}"
        );
        // No content was dropped.
        let joined: String = warn.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
        assert!(joined.contains("Ubuntu 26.04"));
        assert!(joined.contains("comparable across OSes"));
    }

    #[test]
    fn wrap_words_greedy_and_hard_splits_overlong_words() {
        let w = wrap_words("the quick brown fox jumps", 10);
        assert!(w.iter().all(|l| l.chars().count() <= 10));
        assert_eq!(w.join(" "), "the quick brown fox jumps");

        let w = wrap_words("supercalifragilisticexpialidocious tail", 10);
        assert!(w.iter().all(|l| l.chars().count() <= 10));
        assert_eq!(
            w.concat().replace(' ', ""),
            "supercalifragilisticexpialidocioustail"
        );
    }
}

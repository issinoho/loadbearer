//! Human-readable, non-TUI rendering. Used for `info`, `list`, `--plain` output
//! and any run where stdout is not a terminal.

use crate::compare::Comparison;
use crate::engine::Benchmark;
use crate::inventory::Inventory;
use crate::scoring::{Baseline, Grade, PROFILES, ResultFile};

const PKG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Format a byte count as a binary-prefixed size (e.g. `15.0 GiB`).
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn section(title: &str) {
    println!("\n{title}");
}

fn row(key: &str, value: impl AsRef<str>) {
    kv(key, value, 15);
}

fn kv(key: &str, value: impl AsRef<str>, width: usize) {
    println!("  {key:<width$} {}", value.as_ref());
}

pub fn print_inventory(inv: &Inventory) {
    println!("loadbearer {PKG_VERSION} — machine inventory");

    section("Host");
    row("Hostname", inv.hostname.as_deref().unwrap_or("unknown"));
    row("OS", inv.os.as_deref().unwrap_or("unknown"));
    row("Kernel", inv.kernel.as_deref().unwrap_or("unknown"));
    row("Architecture", &inv.arch);

    section("CPU");
    row("Model", &inv.cpu_model);
    row("Vendor", &inv.cpu_vendor);
    let cores = match inv.cpu_physical_cores {
        Some(p) => format!("{p} physical / {} logical", inv.cpu_logical_cores),
        None => format!("{} logical", inv.cpu_logical_cores),
    };
    row("Cores", cores);
    if inv.cpu_mhz_spot > 0 {
        row(
            "Frequency",
            format!("{} MHz (spot reading)", inv.cpu_mhz_spot),
        );
    }

    section("Memory");
    row("RAM", human_bytes(inv.ram_bytes));
    row("Swap", human_bytes(inv.swap_bytes));

    section("Disks");
    if inv.disks.is_empty() {
        row("", "none detected");
    } else {
        for d in &inv.disks {
            println!(
                "  {:<20} {:<8} {:<8} {:>10} total, {:>10} free{}",
                truncate(&d.mount_point, 20),
                truncate(&d.file_system, 8),
                d.kind,
                human_bytes(d.total_bytes),
                human_bytes(d.available_bytes),
                if d.removable { "  (removable)" } else { "" },
            );
        }
    }
    println!();
}

/// Print the catalog of benchmarks, the active baseline and scoring profiles.
pub fn print_catalog(benchmarks: &[Box<dyn Benchmark>], baseline: &Baseline) {
    println!("loadbearer {PKG_VERSION}");

    section("Benchmarks");
    for bench in benchmarks {
        let subtests: Vec<&str> = bench.subtests().iter().map(|s| s.label).collect();
        kv(bench.id(), subtests.join(", "), 9);
    }

    section("Baseline");
    kv(&baseline.name, &baseline.description, 16);

    section("Profiles");
    for p in PROFILES {
        kv(p.name, p.description, 17);
    }
    println!();
}

/// A short bar for a score, where 1000 (baseline) fills about two-thirds.
fn score_bar(score: f64) -> String {
    const WIDTH: usize = 24;
    let filled = ((score / 1500.0) * WIDTH as f64)
        .round()
        .clamp(0.0, WIDTH as f64) as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(WIDTH - filled))
}

/// Render a full scored assessment.
pub fn print_scored_report(result: &ResultFile) {
    let m = &result.machine;
    let cfg = &result.config;

    println!("\nloadbearer {} — assessment", result.tool_version);
    println!(
        "\n  Machine   {} · {} · {} threads · {} RAM",
        m.hostname.as_deref().unwrap_or("unknown"),
        m.cpu_model,
        m.cpu_logical_cores,
        human_bytes(m.ram_bytes),
    );
    println!(
        "  Profile   {} · curve k={} · baseline {} · {} preset",
        cfg.profile, cfg.curve_k, cfg.baseline, cfg.duration_preset,
    );

    for c in &result.components {
        println!(
            "\n  {:<8} {:>6.0}  {}   {}",
            c.label.to_uppercase(),
            c.score,
            grade_tag(c.grade),
            score_bar(c.score),
        );
        for st in &c.subtests {
            println!(
                "    {:<24} {:>11.1} {:<8} {:>6.2}x  {:>6.0}  {}",
                st.label,
                st.value,
                st.unit,
                st.ratio,
                st.score,
                st.confidence.as_str(),
            );
        }
        for note in &c.notes {
            println!("    note: {note}");
        }
    }

    println!(
        "\n  {:<8} {:>6.0}  {}   {}",
        "OVERALL",
        result.overall.score,
        grade_tag(result.overall.grade),
        score_bar(result.overall.score),
    );
    println!("\n  Why:");
    for line in &result.overall.why {
        println!("    - {line}");
    }
    println!(
        "\n  A score of 1000 = the {} baseline. Grades: S≥1400 A≥1150 B≥850 C≥600 D≥400.",
        cfg.baseline,
    );

    if let Some(link) = &result.link {
        println!("\n  Link to {} (measured, not graded)", link.target);
        println!(
            "    {:<20} {:>10.2} GiB/s",
            "TCP upload", link.tcp_upload_gibps
        );
        println!("    {:<20} {:>10.1} us", "TCP round-trip", link.tcp_rtt_us);
        println!(
            "    {:<20} {:>10.1} Kpps",
            "UDP send rate", link.udp_send_kpps
        );
    }
}

fn grade_tag(g: Grade) -> String {
    format!("[{}]", g.as_str())
}

const CMP_NAME_W: usize = 32;
const CMP_COL_W: usize = 15;

/// Render a head-to-head comparison of two or more result files.
pub fn print_comparison(c: &Comparison) {
    let n = c.machines.len();
    println!("\nloadbearer {PKG_VERSION} — compare ({n} machines)\n");

    for m in &c.machines {
        println!("  {}  {:<22} {}", m.tag, m.label, m.cpu_model);
        println!(
            "     {} preset · baseline {} · k={} · stored overall {:.0} [{}]  ({})",
            m.duration_preset,
            m.baseline,
            m.curve_k,
            m.stored_overall_score,
            m.stored_overall_grade,
            m.path,
        );
    }

    if !c.warnings.is_empty() {
        println!();
        for w in &c.warnings {
            println!("  ! {w}");
        }
    }

    print!("\n  {:<CMP_NAME_W$}", "metric");
    for m in &c.machines {
        print!(
            " {:>CMP_COL_W$}",
            truncate(&format!("{}: {}", m.tag, m.label), CMP_COL_W)
        );
    }
    println!();

    for comp in &c.components {
        println!("\n  {}", comp.label.to_uppercase());
        for st in &comp.subtests {
            print!(
                "  {:<CMP_NAME_W$}",
                truncate(&format!("{} ({})", st.label, st.unit), CMP_NAME_W)
            );
            for (mi, value) in st.values.iter().enumerate() {
                let cell = if mi == 0 {
                    fmt_val(*value)
                } else {
                    format!("{} {:+.0}%", fmt_val(*value), (st.rel[mi] - 1.0) * 100.0)
                };
                print!(" {cell:>CMP_COL_W$}");
            }
            println!("  {}", winner_tag(c, &st.rel, st.best));
        }
        print!("  {:<CMP_NAME_W$}", "  \u{2192} component");
        for (mi, rel) in comp.rel.iter().enumerate() {
            print!(" {:>CMP_COL_W$}", rel_cell(mi, *rel));
        }
        println!("  {}", winner_tag(c, &comp.rel, comp.best));
    }

    print!("\n  {:<CMP_NAME_W$}", "OVERALL");
    for (mi, rel) in c.overall.rel.iter().enumerate() {
        print!(" {:>CMP_COL_W$}", rel_cell(mi, *rel));
    }
    println!("  {}", winner_tag(c, &c.overall.rel, c.overall.ranking[0]));

    println!("\n  Verdict: {}\n", c.overall.summary);
}

fn rel_cell(machine_index: usize, rel: f64) -> String {
    if machine_index == 0 {
        "ref".to_string()
    } else {
        format!("{:+.0}%", (rel - 1.0) * 100.0)
    }
}

/// Tag of the winning machine, or `=` when the top two are within 2% of each
/// other (no meaningful winner).
fn winner_tag(c: &Comparison, rel: &[f64], best: usize) -> String {
    let mut sorted: Vec<f64> = rel.to_vec();
    sorted.sort_by(|a, b| b.total_cmp(a));
    let ambiguous = sorted.len() < 2 || sorted[1] <= 0.0 || sorted[0] / sorted[1] - 1.0 < 0.02;
    if ambiguous {
        "=".to_string()
    } else {
        c.machines[best].tag.clone()
    }
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let keep: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{keep}…")
    }
}

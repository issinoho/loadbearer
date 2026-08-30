//! Human-readable, non-TUI rendering. Used for `info`, `list`, `--plain` output
//! and any run where stdout is not a terminal.

use crate::compare::Comparison;
use crate::engine::Benchmark;
use crate::inventory::Inventory;
use crate::mem::{MemSnapshot, Source};
use crate::scoring::{Baseline, Grade, PROFILES, ResultFile};
use crate::soak::SoakResult;

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

    if let Some(g) = &inv.gpu {
        section("GPU");
        row("Model", &g.name);
        row(
            "Type",
            if g.integrated {
                "integrated"
            } else {
                "discrete"
            },
        );
        if g.vram_bytes > 0 {
            row("Memory", human_bytes(g.vram_bytes));
        }
        if !g.opencl_version.is_empty() {
            row("OpenCL", &g.opencl_version);
        }
        if g.compute_units > 0 {
            row(
                "Compute units",
                format!("{} @ {} MHz", g.compute_units, g.clock_mhz),
            );
        }
    }

    if let Some(b) = &inv.battery {
        section("Battery");
        let id = [b.vendor.as_deref(), b.model.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        if !id.is_empty() {
            row("Model", id);
        }
        if let Some(t) = &b.technology {
            row("Technology", t);
        }
        row("Charge", format!("{:.0}% ({})", b.charge_pct, b.state));
        match (b.energy_full_wh, b.energy_full_design_wh, b.health_pct) {
            (Some(full), Some(design), Some(h)) => row(
                "Health",
                format!("{full:.1} / {design:.1} Wh design ({h:.0}%)"),
            ),
            (_, _, Some(h)) => row("Health", format!("{h:.0}% of design capacity")),
            _ => row("Health", "unavailable (no design-capacity reading)"),
        }
        if let Some(c) = b.cycle_count {
            row("Cycles", c.to_string());
        }
        if let Some(v) = b.voltage_v {
            row("Voltage", format!("{v:.2} V"));
        }
        if let Some(t) = b.temperature_c {
            row("Temperature", format!("{t:.0} °C"));
        }
    }

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

/// Render a `ps_mem`-style per-program memory table: smallest first, so the
/// heaviest consumers sit right above the grand total.
pub fn print_mem(snap: &MemSnapshot, limit: Option<usize>, swap: bool) {
    const W: usize = 10;
    println!("loadbearer {PKG_VERSION} — memory by program");

    let header = if swap {
        format!(
            " {:>W$} + {:>W$} = {:>W$} + {:>W$}   Program",
            "Private", "Shared", "RAM used", "Swap"
        )
    } else {
        format!(
            " {:>W$} + {:>W$} = {:>W$}   Program",
            "Private", "Shared", "RAM used"
        )
    };
    let rule_w = if swap { 4 * W + 9 } else { 3 * W + 6 };
    println!("\n{header}\n");

    let total = snap.programs.len();
    let shown: &[crate::mem::ProgramMem] = match limit {
        Some(n) if n < total => &snap.programs[total - n..],
        _ => &snap.programs,
    };

    for p in shown {
        let label = if p.processes > 1 {
            format!("{} ({})", p.name, p.processes)
        } else {
            p.name.clone()
        };
        if swap {
            println!(
                " {:>W$} + {:>W$} = {:>W$} + {:>W$}   {label}",
                human_bytes(p.private_bytes),
                human_bytes(p.shared_bytes),
                human_bytes(p.total_bytes()),
                human_bytes(p.swap_bytes),
            );
        } else {
            println!(
                " {:>W$} + {:>W$} = {:>W$}   {label}",
                human_bytes(p.private_bytes),
                human_bytes(p.shared_bytes),
                human_bytes(p.total_bytes()),
            );
        }
    }

    println!(" {}", "-".repeat(rule_w));
    if swap {
        println!(
            " {:>W$}   {:>W$}   {:>W$}   {:>W$}",
            "",
            "",
            human_bytes(snap.ram_total()),
            human_bytes(snap.swap_total()),
        );
    } else {
        println!(" {:>rule_w$}", human_bytes(snap.ram_total()));
    }
    println!(" {}", "=".repeat(rule_w));

    if let Some(n) = limit
        && n < total
    {
        println!(
            " {} smaller program(s) not shown (still in the total).",
            total - n
        );
    }
    match snap.source {
        Source::Pss => {
            println!(" PSS from /proc/<pid>/smaps_rollup — shared pages counted proportionally.");
            if snap.unreadable > 0 {
                println!(
                    " {} process(es) not readable — run as root for the full total.",
                    snap.unreadable,
                );
            }
        }
        Source::WorkingSet => {
            println!(
                " Working set — Windows has no PSS; Shared is an estimate, Swap is not shown."
            );
            if snap.unreadable > 0 {
                println!(
                    " {} process(es) not readable — run elevated for the full total.",
                    snap.unreadable,
                );
            }
        }
    }
    if swap && snap.source == Source::Pss && !snap.has_swap() {
        println!(" No swap in use.");
    }
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
        let tail = if c.graded {
            String::new()
        } else {
            "   · measured, not in the overall".to_string()
        };
        println!(
            "\n  {:<8} {:>6.0}  {}   {}{}",
            c.label.to_uppercase(),
            c.score,
            grade_tag(c.grade),
            score_bar(c.score),
            tail,
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

    let has_graded = result.components.iter().any(|c| c.graded);
    if has_graded {
        println!(
            "\n  {:<8} {:>6.0}  {}   {}",
            "OVERALL",
            result.overall.score,
            grade_tag(result.overall.grade),
            score_bar(result.overall.score),
        );
    } else {
        println!("\n  {:<8} —   no graded components in this run", "OVERALL");
    }
    println!("\n  Why:");
    for line in &result.overall.why {
        println!("    - {line}");
    }
    if has_graded {
        println!(
            "\n  A score of 1000 = the {} baseline. Grades: S≥1400 A≥1150 B≥850 C≥600 D≥400.",
            cfg.baseline,
        );
    }
    let ungraded: Vec<&str> = result
        .components
        .iter()
        .filter(|c| !c.graded)
        .map(|c| c.label.as_str())
        .collect();
    if !ungraded.is_empty() {
        println!(
            "  {} {} measured and shown, but kept out of the overall grade \
             (OS-dependent / optional hardware).",
            ungraded.join(" and "),
            if ungraded.len() == 1 { "is" } else { "are" },
        );
    }

    if !result.model_ref.is_empty() {
        print_model_ref(&result.model_ref);
    }

    if let Some(b) = &result.machine.battery {
        print_battery_block(b);
    }

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

    if let Some(soak) = &result.soak {
        print_soak_block(soak);
    }
}

/// "vs typical hardware" — how the CPU / GPU compare to a healthy example of
/// their model. Not graded.
fn print_model_ref(refs: &[crate::scoring::models::ModelRef]) {
    println!("\n  vs typical hardware");
    for r in refs {
        let deltas: Vec<String> = r
            .subtests
            .iter()
            .map(|s| format!("{} {:+.0}%", s.id, s.delta_pct))
            .collect();
        println!(
            "    {:<4} {:+.0}%   {}",
            r.component.to_uppercase(),
            r.delta_pct,
            r.verdict,
        );
        println!("      {}", deltas.join(" · "));
        if r.samples <= 1 {
            println!(
                "      (single sample from {} — indicative; add yours with `loadbearer models --add`)",
                r.measured,
            );
        }
    }
}

/// The battery-health block at the foot of a `run` report. Not graded — pack
/// wear is a property of the consumable, not of the silicon under test.
fn print_battery_block(b: &crate::battery::BatteryInfo) {
    println!("\n  {:<8} {} · not graded", "BATTERY", b.summary());
    println!("    {:<12} {:.0}% ({})", "Charge", b.charge_pct, b.state);
    match (b.energy_full_wh, b.energy_full_design_wh, b.health_pct) {
        (Some(full), Some(design), Some(h)) => {
            println!(
                "    {:<12} {full:.1} / {design:.1} Wh design ({h:.0}%)",
                "Health"
            )
        }
        (_, _, Some(h)) => println!("    {:<12} {h:.0}% of design capacity", "Health"),
        _ => println!(
            "    {:<12} unavailable (no design-capacity reading)",
            "Health"
        ),
    }
    if let Some(c) = b.cycle_count {
        println!("    {:<12} {c}", "Cycles");
    }
    if let Some(v) = b.voltage_v {
        println!("    {:<12} {v:.2} V", "Voltage");
    }
    if let Some(t) = b.temperature_c {
        println!("    {:<12} {t:.0} \u{b0}C", "Temperature");
    }
    if let Some(v) = b.health_verdict() {
        println!("    \u{2192} {v}");
    }
    if let Some(n) = b.power_note() {
        println!("    note: {n}");
    }
}

/// Standalone `loadbearer soak` report: a machine line and the soak block.
pub fn print_soak_report(m: &Inventory, s: &SoakResult) {
    println!("\nloadbearer {PKG_VERSION} — soak test");
    println!(
        "\n  Machine   {} · {} · {} threads · {} RAM",
        m.hostname.as_deref().unwrap_or("unknown"),
        m.cpu_model,
        m.cpu_logical_cores,
        human_bytes(m.ram_bytes),
    );
    print_soak_block(s);
    println!(
        "\n  Sustained load is not part of any grade. \"Retained\" is steady-state \
         throughput as a\n  fraction of the machine's own unthrottled peak — how well it \
         holds up under a long\n  workload, not how fast it is."
    );
}

/// The soak result block, shared by the standalone report and `run --soak`.
pub fn print_soak_block(s: &SoakResult) {
    println!(
        "\n  {:<8} {:>3.0}s · {} threads · not graded",
        "SOAK", s.duration_secs, s.threads,
    );
    println!(
        "    {:<12} {:>11.0} {}   ({:.0}\u{2013}{:.0}s)",
        "Peak", s.peak_rate, s.unit, s.peak_window.0, s.peak_window.1,
    );
    println!(
        "    {:<12} {:>11.0} {}   ({:.0}\u{2013}{:.0}s)   {:.1}% retained",
        "Steady", s.steady_rate, s.unit, s.steady_window.0, s.steady_window.1, s.retained_pct,
    );
    match s.onset_secs {
        Some(t) => println!(
            "    {:<12} onset ~{:.0}s (first sustained drop below 95% of peak)",
            "Throttle", t,
        ),
        None => println!(
            "    {:<12} none \u{2014} throughput stayed above 95% of peak",
            "Throttle",
        ),
    }
    println!(
        "    {:<12} steady-window CV {:.1}%",
        "Stability", s.steady_cv_pct,
    );
    if s.mhz_peak > 0 {
        println!(
            "    {:<12} {:.2} GHz peak \u{2192} {:.2} GHz steady",
            "Clock",
            s.mhz_peak as f64 / 1000.0,
            s.mhz_steady as f64 / 1000.0,
        );
    }
    let spark = soak_sparkline(s, 60);
    if !spark.is_empty() {
        println!("    {:<12} {}  (\u{2248}1s/mark)", "Trace", spark);
    }
    println!("    \u{2192} {}", s.verdict());
}

/// A compact Unicode sparkline of the soak throughput samples, bucket-averaged
/// down to at most `max_w` marks.
fn soak_sparkline(s: &SoakResult, max_w: usize) -> String {
    const TICKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let rates: Vec<f64> = s.samples.iter().map(|x| x.rate).collect();
    if rates.is_empty() {
        return String::new();
    }
    let reduced: Vec<f64> = if rates.len() <= max_w {
        rates
    } else {
        let bucket = rates.len().div_ceil(max_w);
        rates
            .chunks(bucket)
            .map(|c| c.iter().sum::<f64>() / c.len() as f64)
            .collect()
    };
    let lo = reduced.iter().copied().fold(f64::MAX, f64::min);
    let hi = reduced.iter().copied().fold(f64::MIN, f64::max);
    let span = (hi - lo).max(1e-9);
    reduced
        .iter()
        .map(|&v| {
            let idx = (((v - lo) / span) * (TICKS.len() - 1) as f64).round() as usize;
            TICKS[idx.min(TICKS.len() - 1)]
        })
        .collect()
}

fn grade_tag(g: Grade) -> String {
    format!("[{}]", g.as_str())
}

const CMP_NAME_W: usize = 32;
const CMP_COL_W: usize = 17;

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

    let body_w = CMP_NAME_W + c.machines.len() * (CMP_COL_W + 1);

    for comp in &c.components {
        println!("\n  {}", comp.label.to_uppercase());
        for st in &comp.subtests {
            print!(
                "  {:<CMP_NAME_W$}",
                truncate(&format!("{} ({})", st.label, st.unit), CMP_NAME_W)
            );
            for (mi, value) in st.values.iter().enumerate() {
                let delta = (mi != 0).then(|| (st.rel[mi] - 1.0) * 100.0);
                print!(" {}", cmp_cell(&fmt_val(*value), delta));
            }
            println!("     {}", winner_tag(c, &st.rel, st.best));
        }
        println!("  {}", "\u{2500}".repeat(body_w));
        print!("  {:<CMP_NAME_W$}", format!("{} total", comp.label));
        for (mi, rel) in comp.rel.iter().enumerate() {
            print!(" {}", rollup_cell(mi, *rel));
        }
        println!("     {}", winner_tag(c, &comp.rel, comp.best));
    }

    println!("\n  {}", "\u{2550}".repeat(body_w));
    print!("  {:<CMP_NAME_W$}", "OVERALL");
    for (mi, rel) in c.overall.rel.iter().enumerate() {
        print!(" {}", rollup_cell(mi, *rel));
    }
    println!(
        "     {}",
        winner_tag(c, &c.overall.rel, c.overall.ranking[0])
    );

    if let Some(sk) = &c.soak {
        println!("\n  SUSTAINED LOAD (not graded)");
        print!("  {:<CMP_NAME_W$}", format!("  steady ({})", sk.unit));
        for (mi, v) in sk.steady_rate.iter().enumerate() {
            let delta = (mi != 0).then(|| (sk.rel_steady[mi] - 1.0) * 100.0);
            print!(" {}", cmp_cell(&fmt_val(*v), delta));
        }
        println!("  {}", c.machines[sk.best_sustained].tag);
        print!("  {:<CMP_NAME_W$}", "  retained vs own peak");
        for v in &sk.retained_pct {
            print!(" {:>CMP_COL_W$}", format!("{v:.0}%"));
        }
        println!("  {}", c.machines[sk.best_retention].tag);
    }

    println!("\n  Verdict: {}\n", c.overall.summary);
}

/// A comparison cell, `CMP_COL_W` wide: the value right-aligned in a left
/// sub-field, then the `±%` delta right-aligned in its own 6-char sub-field, so
/// the deltas line up in a column of their own instead of crowding the number.
fn cmp_cell(value: &str, delta: Option<f64>) -> String {
    const DW: usize = 6;
    let vw = CMP_COL_W - DW;
    match delta {
        Some(pct) => {
            // A delta that rounds to zero prints as a plain `0%`, not `-0%`.
            let d = if pct.round() == 0.0 {
                "0%".to_string()
            } else {
                format!("{pct:+.0}%")
            };
            format!("{value:>vw$}{d:>DW$}")
        }
        None => format!("{value:>vw$}{:DW$}", ""),
    }
}

/// The cell for a rollup row: `ref` for machine 0, otherwise just the `±%`
/// delta, positioned in the same delta sub-column as [`cmp_cell`].
fn rollup_cell(machine_index: usize, rel: f64) -> String {
    if machine_index == 0 {
        cmp_cell("ref", None)
    } else {
        cmp_cell("", Some((rel - 1.0) * 100.0))
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

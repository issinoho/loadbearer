use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Benchmark thoroughness. Trades wall-clock time for lower measurement variance.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[value(rename_all = "lower")]
pub enum DurationArg {
    /// ~10 s per benchmark; for quick checks and CI.
    Short,
    /// ~30 s per benchmark; the default.
    #[default]
    Normal,
    /// ~2 min per benchmark; for a considered assessment.
    Thorough,
}

/// Benchmark and grade a machine's CPU, memory and disk.
///
/// Run the same build on two machines and compare their result files to see
/// which is stronger and why.
#[derive(Parser, Debug)]
#[command(name = "loadbearer", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show machine inventory without running any benchmark.
    Info(InfoArgs),
    /// List available benchmarks, the active baseline and scoring profiles.
    List,
    /// Run benchmarks and produce a graded result.
    Run(RunArgs),
    /// Compare two or more result files and explain the winner.
    Compare(CompareArgs),
    /// Print the built-in baseline, or generate one by averaging result files.
    Baseline(BaselineArgs),
}

#[derive(Args, Debug)]
pub struct InfoArgs {
    /// Emit machine-readable JSON instead of formatted text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Restrict to a subset of benchmarks (comma-separated: cpu,memory,disk).
    #[arg(long, value_delimiter = ',', value_name = "LIST")]
    pub only: Vec<String>,

    /// Scoring profile that weights the overall grade [default: general].
    #[arg(long, value_name = "NAME")]
    pub profile: Option<String>,

    /// Benchmark thoroughness [default: normal].
    #[arg(long, value_enum)]
    pub duration: Option<DurationArg>,

    /// Display-curve exponent; lower values compress the top end [default: 0.5].
    #[arg(long = "curve-k", value_name = "FLOAT")]
    pub curve_k: Option<f64>,

    /// Directory for the disk benchmark's scratch file (defaults to the working directory).
    #[arg(long, value_name = "PATH")]
    pub target_dir: Option<PathBuf>,

    /// Override the number of timed iterations per subtest.
    #[arg(long, value_name = "N")]
    pub runs: Option<u32>,

    /// Seed for reproducible workload generation.
    #[arg(long, value_name = "N")]
    pub seed: Option<u64>,

    /// Write the result JSON to a file.
    #[arg(long, value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Disable the TUI and emit structured lines (implied when stdout is not a terminal).
    #[arg(long)]
    pub plain: bool,

    /// Disable the TUI and emit only the result JSON to stdout.
    #[arg(long, conflicts_with = "plain")]
    pub json: bool,

    /// Load defaults from a config file; explicit switches still win.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct BaselineArgs {
    /// Result files to average into a new baseline. With none, prints the
    /// built-in `reference-v1` baseline unchanged.
    #[arg(value_name = "FILE")]
    pub files: Vec<PathBuf>,

    /// Name for the generated baseline.
    #[arg(long, default_value = "custom", value_name = "NAME")]
    pub name: String,

    /// One-line description for the generated baseline.
    #[arg(long, value_name = "TEXT")]
    pub description: Option<String>,
}

#[derive(Args, Debug)]
pub struct CompareArgs {
    /// Result files to compare (two or more).
    #[arg(required = true, num_args = 2.., value_name = "FILE")]
    pub files: Vec<PathBuf>,

    /// Emit a plain-text table instead of the TUI.
    #[arg(long)]
    pub plain: bool,

    /// Emit the comparison as JSON.
    #[arg(long, conflicts_with = "plain")]
    pub json: bool,
}

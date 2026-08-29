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
#[command(name = "loadbearer", version = env!("LOADBEARER_VERSION"), about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Never touch the GPU: skip the `gpu` component and the OpenCL probe that
    /// `info` and `run` otherwise perform (so `OpenCL.dll` is never loaded).
    #[arg(long, global = true)]
    pub no_gpu: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Show machine inventory without running any benchmark.
    Info(InfoArgs),
    /// Per-program memory use, in the style of `ps_mem` — grouped by program,
    /// largest last, with a grand total. Linux reports true PSS; Windows the
    /// working set. A diagnostic, not a benchmark.
    Mem(MemArgs),
    /// List available benchmarks, the active baseline and scoring profiles.
    List,
    /// Run benchmarks and produce a graded result.
    Run(RunArgs),
    /// Compare two or more result files and explain the winner.
    Compare(CompareArgs),
    /// Re-score an existing result file against a different baseline, profile
    /// or curve — no benchmarks are re-run.
    Score(ScoreArgs),
    /// Print the built-in baseline, or generate one by averaging result files.
    Baseline(BaselineArgs),
    /// Hold every core under sustained load and report throughput retention
    /// (thermal / power-limit throttling). Not scored.
    Soak(SoakArgs),
    /// Run the server side of the `--net-target` link test until stopped.
    NetServer(NetServerArgs),
}

#[derive(Args, Debug)]
pub struct SoakArgs {
    /// Sustained-load duration in seconds [default: 90; range 15–1800].
    #[arg(long, value_name = "SECS")]
    pub duration: Option<u64>,

    /// Worker threads [default: one per logical CPU].
    #[arg(long, value_name = "N")]
    pub threads: Option<usize>,

    /// Seed for the workload kernel.
    #[arg(long, value_name = "N")]
    pub seed: Option<u64>,

    /// Write the soak result JSON to a file.
    #[arg(long, value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Emit only the result JSON to stdout.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct NetServerArgs {
    /// Address to listen on for both TCP and UDP.
    #[arg(long, default_value = "0.0.0.0:47913", value_name = "ADDR")]
    pub bind: String,
}

#[derive(Args, Debug)]
pub struct InfoArgs {
    /// Emit machine-readable JSON instead of formatted text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct MemArgs {
    /// Show only the N largest programs (the total still covers all of them).
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,

    /// Add a Swap column (Linux only; shows the proportional paged-out size).
    #[arg(long)]
    pub swap: bool,

    /// Emit machine-readable JSON instead of the table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Restrict to a subset of benchmarks (comma-separated: cpu,memory,disk,network,gpu).
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

    /// After the graded run, probe a real link to a `loadbearer net-server` at
    /// this `host:port`. Reported separately and not graded.
    #[arg(long, value_name = "HOST:PORT")]
    pub net_target: Option<String>,

    /// After the graded run, hold every core under sustained load and report
    /// throughput retention. Adds ~90 s. Reported separately and not graded.
    #[arg(long)]
    pub soak: bool,

    /// Duration in seconds for `--soak` [default: 90; range 15–1800].
    #[arg(long, value_name = "SECS", requires = "soak")]
    pub soak_duration: Option<u64>,

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
pub struct ScoreArgs {
    /// A result file written by `loadbearer run --output`.
    #[arg(value_name = "FILE")]
    pub file: PathBuf,

    /// Baseline TOML to score against (as written by `loadbearer baseline`).
    /// Default: the built-in `reference-v1`.
    #[arg(long, value_name = "FILE")]
    pub baseline: Option<PathBuf>,

    /// Scoring profile. Default: the profile recorded in the result file.
    #[arg(long, value_name = "NAME")]
    pub profile: Option<String>,

    /// Display-curve exponent, 0.05–3.0. Default: the value in the file.
    #[arg(long = "curve-k", value_name = "FLOAT")]
    pub curve_k: Option<f64>,

    /// Write the re-scored result as a new JSON file.
    #[arg(long, value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Emit the re-scored result as JSON to stdout instead of a report.
    #[arg(long)]
    pub json: bool,
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

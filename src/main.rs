mod battery;
mod benches;
mod cli;
mod compare;
mod config;
mod engine;
mod gates;
mod identity;
mod inventory;
mod logging;
mod mem;
mod output;
mod run;
mod score;
mod scoring;
mod soak;
mod tags;
mod telemetry;
mod tui;
mod util;

use anyhow::Result;
use clap::Parser;
use log::{LevelFilter, error, info};

use cli::{Cli, Command, LogLevelArg};
use logging::LogTarget;

impl From<LogLevelArg> for LevelFilter {
    fn from(v: LogLevelArg) -> Self {
        match v {
            LogLevelArg::Off => LevelFilter::Off,
            LogLevelArg::Error => LevelFilter::Error,
            LogLevelArg::Warn => LevelFilter::Warn,
            LogLevelArg::Info => LevelFilter::Info,
            LogLevelArg::Debug => LevelFilter::Debug,
            LogLevelArg::Trace => LevelFilter::Trace,
        }
    }
}

impl Cli {
    /// Where the diagnostic log should go, from `--log-file` / `--no-log`.
    fn log_target(&self) -> LogTarget {
        if self.no_log {
            LogTarget::Disabled
        } else if let Some(path) = &self.log_file {
            LogTarget::Path(path.clone())
        } else {
            LogTarget::Default
        }
    }
}

/// Process exit codes. `VERSIONING.md` promises `0` on success and non-zero on
/// failure; these only refine the non-zero side. `2` is left alone because
/// clap already uses it for a usage error.
pub mod exit {
    /// Ran and produced a result. Optional extras may have been skipped — see
    /// the result file's `notes`.
    pub const OK: u8 = 0;
    /// Couldn't produce a result.
    pub const ERROR: u8 = 1;
    /// Ran fine, but the grade came in under `--fail-under`.
    pub const BELOW_THRESHOLD: u8 = 3;
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    if let Some(path) = logging::init(cli.log_target(), cli.log_level.map(Into::into)) {
        info!(
            target: "loadbearer",
            "=== loadbearer {} · {} · pid {} · log {} ===",
            env!("LOADBEARER_VERSION"),
            command_name(&cli.command),
            std::process::id(),
            path.display(),
        );
    }

    match dispatch(cli) {
        Ok(code) => {
            info!(target: "loadbearer", "done (exit {code})");
            std::process::ExitCode::from(code)
        }
        Err(e) => {
            error!(target: "loadbearer", "exiting with error: {e:#}");
            // `{:?}` on an anyhow error is the "Error: … / Caused by: …" form
            // that `fn main() -> Result` used to print for us.
            eprintln!("Error: {e:?}");
            std::process::ExitCode::from(exit::ERROR)
        }
    }
}

/// The subcommand name, for the session header line.
fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Info(_) => "info",
        Command::Mem(_) => "mem",
        Command::List => "list",
        Command::Run(_) => "run",
        Command::Compare(_) => "compare",
        Command::Score(_) => "score",
        Command::Baseline(_) => "baseline",
        Command::Models(_) => "models",
        Command::Soak(_) => "soak",
        Command::NetServer(_) => "net-server",
    }
}

/// Returns the exit code to leave with. Only `run` has anything to say beyond
/// success-or-error, so every other arm reports `OK` and relies on `?`.
fn dispatch(cli: Cli) -> Result<u8> {
    if cli.no_gpu {
        info!(target: "loadbearer", "--no-gpu: GPU probe and component disabled");
        benches::gpu_disable();
    }
    match cli.command {
        Command::Info(args) => {
            let inv = inventory::collect();
            if args.json {
                println!("{}", serde_json::to_string_pretty(&inv)?);
            } else {
                output::print_inventory(&inv);
            }
            Ok(exit::OK)
        }
        Command::Mem(args) => mem::execute(args).map(|()| exit::OK),
        Command::List => {
            output::print_catalog(&benches::all(), &scoring::Baseline::reference_v1());
            Ok(exit::OK)
        }
        Command::Run(args) => run::execute(args),
        Command::Compare(args) => compare::execute(args).map(|()| exit::OK),
        Command::Score(args) => score::execute(args).map(|()| exit::OK),
        Command::Baseline(args) => {
            if args.files.is_empty() {
                print!("{}", scoring::Baseline::embedded_toml());
            } else {
                let toml = scoring::generate_baseline(
                    &args.files,
                    &args.name,
                    args.description.as_deref(),
                )?;
                print!("{toml}");
            }
            Ok(exit::OK)
        }
        Command::Models(args) => scoring::models::execute(args).map(|()| exit::OK),
        Command::Soak(args) => soak::execute(args).map(|()| exit::OK),
        Command::NetServer(args) => benches::net_serve(&args.bind).map(|()| exit::OK),
    }
}

mod battery;
mod benches;
mod cli;
mod compare;
mod config;
mod engine;
mod inventory;
mod logging;
mod mem;
mod output;
mod run;
mod score;
mod scoring;
mod soak;
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

fn main() -> Result<()> {
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

    let result = dispatch(cli);
    if let Err(e) = &result {
        error!(target: "loadbearer", "exiting with error: {e:#}");
    } else {
        info!(target: "loadbearer", "done");
    }
    result
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

fn dispatch(cli: Cli) -> Result<()> {
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
            Ok(())
        }
        Command::Mem(args) => mem::execute(args),
        Command::List => {
            output::print_catalog(&benches::all(), &scoring::Baseline::reference_v1());
            Ok(())
        }
        Command::Run(args) => run::execute(args),
        Command::Compare(args) => compare::execute(args),
        Command::Score(args) => score::execute(args),
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
            Ok(())
        }
        Command::Models(args) => scoring::models::execute(args),
        Command::Soak(args) => soak::execute(args),
        Command::NetServer(args) => benches::net_serve(&args.bind),
    }
}

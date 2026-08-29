mod battery;
mod benches;
mod cli;
mod compare;
mod config;
mod engine;
mod inventory;
mod output;
mod run;
mod score;
mod scoring;
mod soak;
mod tui;
mod util;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.no_gpu {
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
        Command::Soak(args) => soak::execute(args),
        Command::NetServer(args) => benches::net_serve(&args.bind),
    }
}

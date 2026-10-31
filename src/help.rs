//! `loadbearer --help` ends with every command and every one of its options,
//! so one screen answers "what can this do" without a `help <command>` per
//! subcommand. Built from the clap definition at startup, so it cannot describe
//! a flag that does not exist. `-h` stays the short summary.

use std::fmt::Write as _;

use clap::{Arg, ArgAction, Command, CommandFactory};

use crate::cli::Cli;

/// The CLI definition with the full reference attached to `--help`.
pub fn command() -> Command {
    let cmd = Cli::command();
    let reference = reference(&cmd);
    cmd.after_long_help(reference)
}

fn reference(cmd: &Command) -> String {
    let header = *cmd.get_styles().get_header();
    let literal = *cmd.get_styles().get_literal();
    let mut out = format!("{header}All commands and their options:{header:#}\n");
    for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
        let _ = writeln!(out, "\n  {literal}{}{literal:#}", sub.get_name());
        let rows: Vec<(String, String)> = sub
            .get_arguments()
            .filter(|a| !a.is_hide_set() && !a.is_global_set())
            .filter(|a| !matches!(a.get_id().as_str(), "help" | "version"))
            .map(|a| (name(a), describe(a)))
            .collect();
        if rows.is_empty() {
            out.push_str("      (no options)\n");
            continue;
        }
        let width = rows.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
        for (n, d) in rows {
            let pad = width - n.len();
            let _ = writeln!(out, "      {literal}{n}{literal:#}{:pad$}  {d}", "");
        }
    }
    out.push_str("\nGlobal options above apply to every command. `loadbearer help <command>` has the detail.");
    out
}

/// `-o, --output <PATH>`, `--plain`, `<FILES>...` — as clap's own listing spells them.
fn name(arg: &Arg) -> String {
    let value = arg
        .get_value_names()
        .and_then(|v| v.first())
        .map(|v| v.to_string())
        .unwrap_or_else(|| arg.get_id().as_str().to_uppercase());
    let repeats = matches!(arg.get_action(), ArgAction::Append);
    if arg.is_positional() {
        return format!("<{value}>{}", if repeats { "..." } else { "" });
    }
    let mut s = match arg.get_short() {
        Some(c) => format!("-{c}, "),
        None => "    ".to_string(),
    };
    if let Some(long) = arg.get_long() {
        let _ = write!(s, "--{long}");
    }
    if arg.get_action().takes_values() {
        let _ = write!(s, " <{value}>");
    }
    s
}

/// The one-line help, then the default and the choices, as clap's own listing.
fn describe(arg: &Arg) -> String {
    let mut s = arg.get_help().map(|h| h.to_string()).unwrap_or_default();
    let defaults: Vec<String> = arg
        .get_default_values()
        .iter()
        .map(|v| v.to_string_lossy().into_owned())
        .collect();
    if !defaults.is_empty() && arg.get_action().takes_values() {
        let _ = write!(s, " [default: {}]", defaults.join(","));
    }
    let choices: Vec<String> = arg
        .get_possible_values()
        .iter()
        .filter(|p| !p.is_hide_set())
        .map(|p| p.get_name().to_string())
        .collect();
    if !choices.is_empty() && arg.get_action().takes_values() {
        let _ = write!(s, " [possible values: {}]", choices.join(", "));
    }
    s.trim_start().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_names_every_option_of_every_command() {
        let cmd = Cli::command();
        let text = reference(&cmd);
        for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
            assert!(
                text.contains(sub.get_name()),
                "{} is missing",
                sub.get_name()
            );
            for arg in sub
                .get_arguments()
                .filter(|a| !a.is_hide_set() && !a.is_global_set())
            {
                if let Some(long) = arg.get_long().filter(|l| *l != "help") {
                    assert!(
                        text.contains(&format!("--{long}")),
                        "{} --{long} is missing",
                        sub.get_name()
                    );
                }
            }
        }
    }

    #[test]
    fn long_help_carries_the_reference_and_short_help_does_not() {
        let mut cmd = command();
        let long = cmd.render_long_help().to_string();
        let short = cmd.render_help().to_string();
        assert!(long.contains("All commands and their options:"));
        assert!(long.contains("--upload-token-file"));
        assert!(!short.contains("All commands and their options:"));
    }

    #[test]
    fn the_definition_is_still_valid_with_the_reference_attached() {
        command().debug_assert();
    }
}

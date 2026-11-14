//! What is collected only when asked for: `--collect uptime,elevation,security`
//! on the command line, `collect = [...]` in a config file.
//!
//! Everything loadbearer collects by default is hardware and OS inventory —
//! what a model of machine is and what it runs. These say something about one
//! particular machine or the person using it: when it was last booted (how it
//! is used), whether this run was elevated (how it was invoked), and its
//! security posture (a list of machines without Secure Boot or disk encryption
//! is sensitive if a results share or a collector leaks). So they are off
//! unless named.
//!
//! "Off" means the code path does not run, not that a value is read and then
//! dropped: [`gather`] is the only caller of the collectors, and it calls each
//! one only behind its name. With nothing named it returns at once, reads
//! nothing and logs nothing. The test module checks that with a fake
//! [`Sources`] that counts its calls.
//!
//! What was asked for is written into the result as `run.collect`, so a reader
//! can tell "not asked" (the name is missing) from "could not be read" (the
//! name is there and the block or field is not).

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;

use serde::{Deserialize, Serialize};

use crate::boot::Boot;
use crate::cli::CollectArg;
use crate::security::Security;

/// One opt-in category. The serialised names are the config-file and
/// `run.collect` spellings, and match the `--collect` values (tested).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Item {
    /// `machine.boot`: when the machine last booted and how long it has been up.
    Uptime,
    /// `run.elevated`: whether this run had administrator / root rights.
    Elevation,
    /// `machine.security`: firmware mode, Secure Boot, TPM, disk encryption.
    Security,
}

impl Item {
    pub fn name(self) -> &'static str {
        match self {
            Item::Uptime => "uptime",
            Item::Elevation => "elevation",
            Item::Security => "security",
        }
    }
}

impl From<CollectArg> for Item {
    fn from(value: CollectArg) -> Self {
        match value {
            CollectArg::Uptime => Item::Uptime,
            CollectArg::Elevation => Item::Elevation,
            CollectArg::Security => Item::Security,
        }
    }
}

/// The set of categories asked for, de-duplicated and in a fixed order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice(BTreeSet<Item>);

impl Choice {
    pub fn of(items: impl IntoIterator<Item = Item>) -> Self {
        Choice(items.into_iter().collect())
    }

    /// The config file's list and the command line's, combined. The command
    /// line **adds to** the config rather than replacing it: a fleet config
    /// sets what every run collects, and someone adding `--collect uptime`
    /// by hand shouldn't silently drop what the fleet asked for. There is no
    /// way to subtract one; leave out `--config` for a run without them.
    pub fn resolve(file: Option<&[Item]>, cli: &[CollectArg]) -> Self {
        Choice::of(
            file.unwrap_or_default()
                .iter()
                .copied()
                .chain(cli.iter().map(|&c| Item::from(c))),
        )
    }

    /// [`resolve`](Self::resolve) for a command that has no other use for the
    /// config file (`soak`, `info`): reads `collect` from it and nothing else,
    /// so one fleet file can set what every command collects. `run` loads the
    /// whole file itself and calls `resolve`. Fails on a file that can't be
    /// read or parsed, as `run` does, before anything is collected.
    pub fn load(config: Option<&Path>, cli: &[CollectArg]) -> Result<Self> {
        let file = match config {
            Some(path) => crate::config::FileConfig::load(path)?,
            None => crate::config::FileConfig::default(),
        };
        Ok(Self::resolve(file.collect.as_deref(), cli))
    }

    pub fn has(&self, item: Item) -> bool {
        self.0.contains(&item)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// In declaration order, for `run.collect`.
    pub fn items(&self) -> Vec<Item> {
        self.0.iter().copied().collect()
    }

    pub fn without(&self, item: Item) -> Self {
        Choice(self.0.iter().copied().filter(|&i| i != item).collect())
    }
}

/// Where the opt-in facts come from. [`Host`] reads this machine; the tests
/// substitute a fake that counts what it was asked.
pub trait Sources {
    fn boot(&self) -> Option<Boot>;
    fn elevated(&self) -> Option<bool>;
    fn security(&self) -> Option<Security>;
}

/// This machine.
pub struct Host;

impl Sources for Host {
    fn boot(&self) -> Option<Boot> {
        crate::boot::read()
    }
    fn elevated(&self) -> Option<bool> {
        crate::elevation::read()
    }
    fn security(&self) -> Option<Security> {
        crate::security::read()
    }
}

/// What [`gather`] found: each field is `None` when not asked for, or when
/// asked for and nothing could be read — `asked` says which.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gathered {
    pub asked: Choice,
    pub boot: Option<Boot>,
    pub elevated: Option<bool>,
    pub security: Option<Security>,
}

/// Read exactly the categories in `choice`, and nothing else.
pub fn gather(choice: &Choice, src: &impl Sources) -> Gathered {
    if choice.is_empty() {
        return Gathered::default();
    }
    log::info!(
        target: "loadbearer::optin",
        "collecting on request: {}",
        choice.items().iter().map(|i| i.name()).collect::<Vec<_>>().join(", "),
    );
    let boot = choice.has(Item::Uptime).then(|| src.boot()).flatten();
    let elevated = choice
        .has(Item::Elevation)
        .then(|| src.elevated())
        .flatten();
    let security = choice.has(Item::Security).then(|| src.security()).flatten();
    // Whether each was readable — never what it said.
    if choice.has(Item::Uptime) {
        log::debug!(target: "loadbearer::optin", "uptime: {}", readable(boot.is_some()));
    }
    if choice.has(Item::Elevation) {
        log::debug!(target: "loadbearer::optin", "elevation: {}", readable(elevated.is_some()));
    }
    if choice.has(Item::Security) {
        let s = security.clone().unwrap_or_default();
        log::debug!(
            target: "loadbearer::optin",
            "security: firmware_mode={} secure_boot={} tpm={} disk_encryption={}",
            readable(s.firmware_mode.is_some()),
            readable(s.secure_boot.is_some()),
            readable(s.tpm.is_some()),
            readable(s.disk_encryption.is_some()),
        );
    }
    Gathered {
        asked: choice.clone(),
        boot,
        elevated,
        security,
    }
}

fn readable(yes: bool) -> &'static str {
    if yes { "readable" } else { "not readable" }
}

/// "3d 4h", "3h 05m", "12m", "40s": how long, the way a person would say it.
pub fn span(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h {:02}m", secs / 3600, (secs % 3600) / 60),
        _ => format!("{}d {}h", secs / 86400, (secs % 86400) / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::Tpm;
    use clap::ValueEnum;
    use std::cell::Cell;

    /// Returns something for every category and counts what it was asked.
    #[derive(Default)]
    struct Counting {
        boot: Cell<u32>,
        elevated: Cell<u32>,
        security: Cell<u32>,
    }

    impl Sources for Counting {
        fn boot(&self) -> Option<Boot> {
            self.boot.set(self.boot.get() + 1);
            Some(Boot {
                booted_at: "2026-11-13T08:00:00Z".into(),
                uptime_secs: 3600,
            })
        }
        fn elevated(&self) -> Option<bool> {
            self.elevated.set(self.elevated.get() + 1);
            Some(false)
        }
        fn security(&self) -> Option<Security> {
            self.security.set(self.security.get() + 1);
            Some(Security {
                firmware_mode: Some("uefi".into()),
                secure_boot: Some(true),
                tpm: Some(Tpm {
                    present: true,
                    version: None,
                }),
                disk_encryption: None,
            })
        }
    }

    impl Counting {
        fn calls(&self) -> (u32, u32, u32) {
            (self.boot.get(), self.elevated.get(), self.security.get())
        }
    }

    #[test]
    fn the_default_reads_nothing_at_all() {
        let src = Counting::default();
        let g = gather(&Choice::default(), &src);
        assert_eq!(src.calls(), (0, 0, 0));
        assert_eq!(g, Gathered::default());
        assert!(g.asked.is_empty());
    }

    #[test]
    fn each_name_reads_exactly_its_own_category() {
        let cases = [
            (Item::Uptime, (1, 0, 0)),
            (Item::Elevation, (0, 1, 0)),
            (Item::Security, (0, 0, 1)),
        ];
        for (item, calls) in cases {
            let src = Counting::default();
            let g = gather(&Choice::of([item]), &src);
            assert_eq!(src.calls(), calls, "{item:?}");
            assert_eq!(g.boot.is_some(), item == Item::Uptime);
            assert_eq!(g.elevated.is_some(), item == Item::Elevation);
            assert_eq!(g.security.is_some(), item == Item::Security);
            assert_eq!(g.asked.items(), vec![item]);
        }
        let src = Counting::default();
        let all = Choice::of([Item::Security, Item::Uptime, Item::Elevation]);
        let g = gather(&all, &src);
        assert_eq!(src.calls(), (1, 1, 1));
        assert!(g.boot.is_some() && g.elevated.is_some() && g.security.is_some());
    }

    #[test]
    fn the_command_line_adds_to_the_config() {
        let file = [Item::Security];
        let c = Choice::resolve(Some(&file), &[CollectArg::Uptime, CollectArg::Security]);
        assert_eq!(c.items(), vec![Item::Uptime, Item::Security]);
        assert_eq!(
            Choice::resolve(None, &[CollectArg::Elevation]).items(),
            vec![Item::Elevation]
        );
        assert_eq!(
            Choice::resolve(Some(&file), &[]).items(),
            vec![Item::Security]
        );
        assert!(Choice::resolve(None, &[]).is_empty());
        assert!(Choice::resolve(Some(&[]), &[]).is_empty());
    }

    #[test]
    fn cli_config_and_document_spell_the_names_alike() {
        for arg in CollectArg::value_variants() {
            let cli_name = arg.to_possible_value().unwrap().get_name().to_string();
            let item = Item::from(*arg);
            assert_eq!(cli_name, item.name());
            assert_eq!(serde_json::to_value(item).unwrap(), cli_name);
        }
        assert_eq!(CollectArg::value_variants().len(), 3);
    }

    fn parse(args: &[&str]) -> Result<crate::cli::Cli, clap::Error> {
        use clap::FromArgMatches;
        let m = crate::help::command().try_get_matches_from(args)?;
        crate::cli::Cli::from_arg_matches(&m)
    }

    #[test]
    fn the_flag_takes_lists_and_repeats_and_refuses_a_typo() {
        use crate::cli::Command;
        let cli = parse(&[
            "loadbearer",
            "run",
            "--collect",
            "uptime,security",
            "--collect",
            "elevation",
        ])
        .unwrap();
        let Command::Run(run) = cli.command else {
            panic!("not run");
        };
        assert_eq!(
            Choice::resolve(None, &run.collect).items(),
            vec![Item::Uptime, Item::Elevation, Item::Security]
        );
        let cli = parse(&["loadbearer", "info", "--collect", "uptime"]).unwrap();
        let Command::Info(info) = cli.command else {
            panic!("not info");
        };
        assert_eq!(info.collect, vec![CollectArg::Uptime]);
        // Not given: nothing.
        let Command::Run(run) = parse(&["loadbearer", "run"]).unwrap().command else {
            panic!("not run");
        };
        assert!(run.collect.is_empty());

        for cmd in ["run", "info"] {
            let err = parse(&["loadbearer", cmd, "--collect", "uptime,serial"])
                .unwrap_err()
                .to_string();
            for name in ["serial", "uptime", "elevation", "security"] {
                assert!(err.contains(name), "{err}");
            }
        }
    }

    #[test]
    fn soak_takes_the_same_flags_and_refuses_the_same_typo() {
        use crate::cli::Command;
        let Command::Soak(soak) = parse(&[
            "loadbearer",
            "soak",
            "--collect",
            "uptime,security",
            "--config",
            "lb.toml",
        ])
        .unwrap()
        .command
        else {
            panic!("not soak");
        };
        assert_eq!(soak.collect, vec![CollectArg::Uptime, CollectArg::Security]);
        assert!(soak.config.is_some());
        let Command::Soak(soak) = parse(&["loadbearer", "soak"]).unwrap().command else {
            panic!("not soak");
        };
        assert!(soak.collect.is_empty() && soak.config.is_none());
        assert!(parse(&["loadbearer", "soak", "--collect", "serial"]).is_err());
    }

    /// A config file holding `text`, in a directory of its own.
    fn config_file(tag: &str, text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lb-optin-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lb.toml");
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn load_reads_collect_from_the_config_and_the_flag_adds_to_it() {
        let path = config_file(
            "load",
            "collect = [\"uptime\"]
profile = \"general\"
",
        );
        let both = Choice::load(Some(&path), &[CollectArg::Security]).unwrap();
        assert_eq!(both.items(), vec![Item::Uptime, Item::Security]);
        let file_only = Choice::load(Some(&path), &[]).unwrap();
        assert_eq!(file_only.items(), vec![Item::Uptime]);
        // No config and no flag: nothing is asked.
        assert!(Choice::load(None, &[]).unwrap().is_empty());
        assert_eq!(
            Choice::load(None, &[CollectArg::Elevation])
                .unwrap()
                .items(),
            vec![Item::Elevation]
        );
        // A config with no collect key asks for nothing.
        let silent = config_file(
            "silent",
            "profile = \"general\"
",
        );
        assert!(Choice::load(Some(&silent), &[]).unwrap().is_empty());
    }

    #[test]
    fn load_fails_on_a_config_it_cannot_read_or_parse() {
        let missing = std::env::temp_dir()
            .join("lb-optin-no-such-dir")
            .join("lb.toml");
        assert!(Choice::load(Some(&missing), &[]).is_err());
        let bad = config_file(
            "bad",
            "collect = [\"serial\"]
",
        );
        assert!(Choice::load(Some(&bad), &[CollectArg::Uptime]).is_err());
    }

    #[test]
    fn span_reads_like_a_person_would_say_it() {
        assert_eq!(span(40), "40s");
        assert_eq!(span(12 * 60 + 5), "12m");
        assert_eq!(span(3 * 3600 + 5 * 60), "3h 05m");
        assert_eq!(span(3 * 86400 + 4 * 3600 + 59), "3d 4h");
    }
}

//! Free-form `key=value` labels attached to a result by whoever ran it.
//!
//! loadbearer can read a machine's hardware but not its place in an
//! organisation — which site it sits at, whose budget bought it, which
//! deployment ring it's in. That context lives in the tool that orchestrates
//! the run (PDQ, SCCM, Ansible, a shell script), so rather than teach
//! loadbearer to query any of them, let the caller pass what it already knows
//! and carry it through to the result file for a collector to group by later.
//!
//! Tags are metadata only: nothing here influences a benchmark, a score or a
//! grade.

use std::collections::BTreeMap;

use anyhow::{Result, bail};

/// A tag set, ordered by key so a result file's JSON is stable between runs.
pub type Tags = BTreeMap<String, String>;

/// Merge file-config tags with `--tag` switches. Switches win per key, rather
/// than replacing the set wholesale: a config file distributed fleet-wide
/// typically carries the constants (site, ring) while the command line adds
/// what's specific to one run (a batch id), and both are worth keeping.
pub fn resolve(from_config: Option<&Tags>, from_cli: &[String]) -> Result<Tags> {
    let mut out = from_config.cloned().unwrap_or_default();
    for raw in from_cli {
        let (k, v) = parse_one(raw)?;
        out.insert(k, v);
    }
    for k in out.keys() {
        validate_key(k)?;
    }
    Ok(out)
}

/// Split one `key=value` switch. The value may itself contain `=`; only the
/// first one separates.
fn parse_one(raw: &str) -> Result<(String, String)> {
    let Some((k, v)) = raw.split_once('=') else {
        bail!("--tag {raw:?} isn't a key=value pair (expected something like --tag site=glasgow)");
    };
    let key = k.trim().to_string();
    let value = v.trim().to_string();
    validate_key(&key)?;
    if value.is_empty() {
        bail!("--tag {raw:?} has an empty value; leave the tag out instead");
    }
    Ok((key, value))
}

/// Keys end up as JSON object keys and as column names in whatever a collector
/// loads them into, so keep them boring.
fn validate_key(key: &str) -> Result<()> {
    if key.is_empty() {
        bail!("a tag key can't be empty");
    }
    if let Some(bad) = key
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
    {
        bail!(
            "tag key {key:?} contains {bad:?}; keys are limited to letters, digits, '_', '-' and '.'"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(pairs: &[&str]) -> Vec<String> {
        pairs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_pairs_from_the_command_line() {
        let t = resolve(None, &cli(&["site=glasgow", "dept=engineering"])).unwrap();
        assert_eq!(t.get("site").map(String::as_str), Some("glasgow"));
        assert_eq!(t.get("dept").map(String::as_str), Some("engineering"));
    }

    #[test]
    fn a_value_may_contain_equals_signs() {
        let t = resolve(None, &cli(&["note=a=b=c"])).unwrap();
        assert_eq!(t.get("note").map(String::as_str), Some("a=b=c"));
    }

    #[test]
    fn switches_override_config_per_key_and_keep_the_rest() {
        let mut cfg = Tags::new();
        cfg.insert("site".into(), "glasgow".into());
        cfg.insert("ring".into(), "pilot".into());

        let t = resolve(Some(&cfg), &cli(&["ring=broad", "batch=7"])).unwrap();
        assert_eq!(t.get("site").map(String::as_str), Some("glasgow"));
        assert_eq!(t.get("ring").map(String::as_str), Some("broad"));
        assert_eq!(t.get("batch").map(String::as_str), Some("7"));
    }

    #[test]
    fn later_switches_win_over_earlier_ones() {
        let t = resolve(None, &cli(&["ring=pilot", "ring=broad"])).unwrap();
        assert_eq!(t.get("ring").map(String::as_str), Some("broad"));
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        let t = resolve(None, &cli(&[" site = glasgow "])).unwrap();
        assert_eq!(t.get("site").map(String::as_str), Some("glasgow"));
    }

    #[test]
    fn a_pair_without_an_equals_sign_is_rejected() {
        let err = resolve(None, &cli(&["site"])).unwrap_err().to_string();
        assert!(err.contains("key=value"), "{err}");
    }

    #[test]
    fn an_empty_value_is_rejected() {
        assert!(resolve(None, &cli(&["site="])).is_err());
    }

    #[test]
    fn a_hostile_key_is_rejected_wherever_it_came_from() {
        assert!(resolve(None, &cli(&["a b=c"])).is_err());
        assert!(resolve(None, &cli(&["a:b=c"])).is_err());
        assert!(resolve(None, &cli(&["=c"])).is_err());

        let mut cfg = Tags::new();
        cfg.insert("not a key".into(), "v".into());
        assert!(resolve(Some(&cfg), &[]).is_err());
    }

    #[test]
    fn dots_dashes_and_underscores_are_fine() {
        assert!(resolve(None, &cli(&["cost.centre-2_a=x"])).is_ok());
    }

    #[test]
    fn no_tags_is_an_empty_set_not_an_error() {
        assert!(resolve(None, &[]).unwrap().is_empty());
    }
}

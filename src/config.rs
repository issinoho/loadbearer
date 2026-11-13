//! Optional TOML config file for `loadbearer run`. Every field is optional;
//! explicit command-line switches always win over file values.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub profile: Option<String>,
    /// "short" | "normal" | "thorough".
    pub duration: Option<String>,
    pub curve_k: Option<f64>,
    pub target_dir: Option<PathBuf>,
    pub runs: Option<u32>,
    pub seed: Option<u64>,
    pub only: Option<Vec<String>>,
    /// `key = "value"` labels carried into the result file. `--tag` wins per
    /// key; see `crate::tags`.
    pub tags: Option<crate::tags::Tags>,
    /// What to collect beyond the default inventory: `uptime`, `elevation`,
    /// `security`. `--collect` adds to this list; see `crate::optin`. A
    /// loadbearer older than this key refuses the whole file, like any
    /// unknown key.
    pub collect: Option<Vec<crate::optin::Item>>,
    /// Where `run` posts its result. `--upload` / `--upload-token-file` win.
    pub upload: Option<UploadConfig>,
}

/// `[upload]`: a loadbearer-fleet server to post the result to.
///
/// A path to the token rather than the token, so a config file that is
/// copied, attached to a ticket or checked in does not carry the credential
/// with it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UploadConfig {
    pub url: Option<String>,
    pub token_file: Option<PathBuf>,
    /// `status = false` stops a skipped run sending its run status record
    /// to this server; absent means on. `--no-status` turns it off too, and
    /// neither can turn it back on. A loadbearer older than this key refuses
    /// the whole file, like any unknown key, so a config carrying it must
    /// not be handed to one.
    pub status: Option<bool>,
}

impl FileConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        let cfg = toml::from_str(&text)
            .with_context(|| format!("parsing config file {}", path.display()))?;
        log::info!(target: "loadbearer::config", "loaded config from {}", path.display());
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_config() {
        let cfg: FileConfig = toml::from_str(
            r#"
            profile = "server"
            duration = "thorough"
            curve_k = 0.4
            only = ["cpu", "disk"]
            runs = 7
            seed = 99
        "#,
        )
        .unwrap();
        assert_eq!(cfg.profile.as_deref(), Some("server"));
        assert_eq!(cfg.duration.as_deref(), Some("thorough"));
        assert_eq!(cfg.curve_k, Some(0.4));
        assert_eq!(cfg.only, Some(vec!["cpu".into(), "disk".into()]));
        assert_eq!(cfg.runs, Some(7));
    }

    #[test]
    fn empty_config_is_all_none() {
        let cfg: FileConfig = toml::from_str("").unwrap();
        assert!(cfg.profile.is_none() && cfg.duration.is_none() && cfg.only.is_none());
    }

    #[test]
    fn unknown_key_is_rejected() {
        assert!(toml::from_str::<FileConfig>("nope = 1").is_err());
    }

    #[test]
    fn parses_a_collect_list() {
        use crate::optin::Item;
        let cfg: FileConfig = toml::from_str("collect = [\"uptime\", \"security\"]").unwrap();
        assert_eq!(cfg.collect, Some(vec![Item::Uptime, Item::Security]));
        let empty: FileConfig = toml::from_str("collect = []").unwrap();
        assert_eq!(empty.collect, Some(vec![]));
    }

    #[test]
    fn an_unknown_collect_name_is_refused_naming_the_valid_ones() {
        let err = toml::from_str::<FileConfig>("collect = [\"uptime\", \"serial\"]")
            .unwrap_err()
            .to_string();
        for name in ["serial", "uptime", "elevation", "security"] {
            assert!(err.contains(name), "{err}");
        }
        // Not a list, and the wrong case, are refused too.
        assert!(toml::from_str::<FileConfig>("collect = \"uptime\"").is_err());
        assert!(toml::from_str::<FileConfig>("collect = [\"Uptime\"]").is_err());
    }

    #[test]
    fn parses_the_status_switch_under_upload_and_nowhere_else() {
        let cfg: FileConfig = toml::from_str(
            "[upload]\nurl = \"https://fleet.example/api/upload\"\nstatus = false\n",
        )
        .unwrap();
        assert_eq!(cfg.upload.unwrap().status, Some(false));
        assert!(toml::from_str::<FileConfig>("status = false").is_err());
        assert!(toml::from_str::<FileConfig>("[upload]\nstatus = \"off\"\n").is_err());
    }

    #[test]
    fn parses_an_upload_section_and_refuses_an_inline_token() {
        let cfg: FileConfig = toml::from_str(
            r#"
            [upload]
            url = "https://fleet.example/api/upload"
            token_file = "C:/ProgramData/loadbearer/fleet-token"
        "#,
        )
        .unwrap();
        let upload = cfg.upload.expect("section");
        assert_eq!(
            upload.url.as_deref(),
            Some("https://fleet.example/api/upload")
        );
        assert!(upload.token_file.is_some());
        assert_eq!(upload.status, None, "on unless said otherwise");

        // There is deliberately no `token` key: the secret goes in a file.
        assert!(toml::from_str::<FileConfig>("[upload]\ntoken = \"lbf_x\"\n").is_err());
    }
}

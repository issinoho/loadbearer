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
}

use directories::ProjectDirs;
use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_cloud_url")]
    pub cloud_url: String,

    #[serde(default = "default_cloud_api_url")]
    pub cloud_api_url: String,

    #[serde(default)]
    pub obs_token: Option<String>,

    #[serde(default)]
    pub obs_code: Option<String>,

    #[serde(default)]
    pub match_id: Option<String>,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_log_level")]
    pub log_level: String,
}

fn default_cloud_url() -> String {
    "wss://chukka.app/ws".to_string()
}

fn default_cloud_api_url() -> String {
    "https://chukka.app/api/v1".to_string()
}

fn default_port() -> u16 {
    4747
}

fn default_log_level() -> String {
    "info".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cloud_url: default_cloud_url(),
            cloud_api_url: default_cloud_api_url(),
            obs_token: None,
            obs_code: None,
            match_id: None,
            port: default_port(),
            log_level: default_log_level(),
        }
    }
}

impl Config {
    pub fn project_dirs() -> Option<ProjectDirs> {
        ProjectDirs::from("app", "chukka", "chukka-obs")
    }

    pub fn config_dir() -> Option<PathBuf> {
        Self::project_dirs().map(|d| d.config_dir().to_path_buf())
    }

    pub fn config_path() -> Option<PathBuf> {
        Self::config_dir().map(|d| d.join("config.toml"))
    }

    pub fn log_dir() -> Option<PathBuf> {
        Self::project_dirs().map(|d| d.data_dir().join("logs"))
    }

    /// Load config from disk, layered: defaults → TOML file → environment.
    pub fn load(path_override: Option<&PathBuf>) -> Self {
        let mut figment = Figment::from(Serialized::defaults(Config::default()));

        let path = path_override
            .cloned()
            .or_else(Self::config_path);

        if let Some(ref p) = path {
            if p.exists() {
                figment = figment.merge(Toml::file(p));
            }
        }

        figment = figment.merge(Env::prefixed("CHUKKA_"));

        figment.extract().unwrap_or_default()
    }

    /// Persist current config to disk.
    pub fn save(&self) -> anyhow::Result<()> {
        let dir = Self::config_dir()
            .ok_or_else(|| anyhow::anyhow!("cannot determine config directory"))?;

        std::fs::create_dir_all(&dir)?;

        let path = dir.join("config.toml");
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;

        Ok(())
    }

    pub fn is_configured(&self) -> bool {
        self.match_id.is_some() && (self.obs_token.is_some() || self.obs_code.is_some())
    }

    /// Build the full WebSocket URL for connecting to chukka-cloud.
    pub fn ws_url(&self) -> Option<String> {
        let match_id = self.match_id.as_deref()?;

        if let Some(code) = &self.obs_code {
            return Some(format!(
                "{}/match/{}?code={}",
                self.cloud_url, match_id, code
            ));
        }

        if let Some(token) = &self.obs_token {
            return Some(format!(
                "{}/match/{}?token={}",
                self.cloud_url, match_id, token
            ));
        }

        None
    }

}

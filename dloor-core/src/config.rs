use std::{
    fs,
    path::{Path, PathBuf},
};

use directories::{ProjectDirs, UserDirs};

use crate::{download::Quality, Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Destination {
    Local { path: PathBuf },
    Cloud { remote: String, path: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Config {
    pub destination: Destination,
    pub default_quality: Quality,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            destination: Destination::Local {
                path: default_download_dir(),
            },
            default_quality: Quality::Best,
        }
    }
}

impl Config {
    pub fn config_path() -> Result<PathBuf> {
        let dirs =
            ProjectDirs::from("com", "dloor", "dloor-tui").ok_or(Error::ConfigDirUnavailable)?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    pub fn exists() -> bool {
        Self::config_path().is_ok_and(|path| path.exists())
    }

    pub fn load() -> Result<Self> {
        Self::load_from(Self::config_path()?)
    }

    pub fn load_or_default() -> Result<Self> {
        let path = Self::config_path()?;
        if path.exists() {
            Self::load_from(path)
        } else {
            Ok(Self::default())
        }
    }

    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let raw = fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::config_path()?;
        self.save_to(&path)?;
        Ok(path)
    }

    pub fn save_to(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self)?;
        fs::write(path, raw)?;
        Ok(())
    }
}

pub fn default_download_dir() -> PathBuf {
    UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = Config {
            destination: Destination::Cloud {
                remote: "gdrive".to_string(),
                path: "videos".to_string(),
            },
            default_quality: Quality::Compressed,
        };

        config.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), config);
    }
}

use std::{
    ffi::OsString,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Browser {
    Chrome,
    Firefox,
    Safari,
    Edge,
    Brave,
    Chromium,
    Vivaldi,
    Opera,
}

impl Browser {
    pub const ALL: [Self; 8] = [
        Self::Chrome,
        Self::Firefox,
        Self::Safari,
        Self::Edge,
        Self::Brave,
        Self::Chromium,
        Self::Vivaldi,
        Self::Opera,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Firefox => "Firefox",
            Self::Safari => "Safari",
            Self::Edge => "Edge",
            Self::Brave => "Brave",
            Self::Chromium => "Chromium",
            Self::Vivaldi => "Vivaldi",
            Self::Opera => "Opera",
        }
    }

    pub fn yt_dlp_name(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Firefox => "firefox",
            Self::Safari => "safari",
            Self::Edge => "edge",
            Self::Brave => "brave",
            Self::Chromium => "chromium",
            Self::Vivaldi => "vivaldi",
            Self::Opera => "opera",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CookieSource {
    #[default]
    None,
    Browser {
        browser: Browser,
    },
    File {
        path: PathBuf,
    },
}

impl CookieSource {
    pub fn yt_dlp_args(&self) -> Vec<OsString> {
        match self {
            Self::None => Vec::new(),
            Self::Browser { browser } => vec![
                "--cookies-from-browser".into(),
                browser.yt_dlp_name().into(),
            ],
            Self::File { path } => vec!["--cookies".into(), path.as_os_str().to_os_string()],
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::None => "public content only".to_string(),
            Self::Browser { browser } => format!("{} browser session", browser.label()),
            Self::File { .. } => "cookie file".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Config {
    pub destination: Destination,
    pub default_quality: Quality,
    pub cookies: CookieSource,
}

#[derive(serde::Deserialize)]
struct ConfigFile {
    destination: Destination,
    default_quality: Quality,
    #[serde(default)]
    cookies: Option<CookieSource>,
    #[serde(default)]
    browser: Option<Browser>,
}

impl<'de> serde::Deserialize<'de> for Config {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let file = ConfigFile::deserialize(deserializer)?;
        Ok(Self {
            destination: file.destination,
            default_quality: file.default_quality,
            cookies: file.cookies.unwrap_or_else(|| {
                file.browser
                    .map_or(CookieSource::None, |browser| CookieSource::Browser {
                        browser,
                    })
            }),
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            destination: Destination::Local {
                path: default_download_dir(),
            },
            default_quality: Quality::Best,
            cookies: CookieSource::None,
        }
    }
}

pub(crate) fn sanitized_ytdlp_error(stderr: &str, cookies: &CookieSource) -> String {
    let lower = stderr.to_ascii_lowercase();
    match cookies {
        CookieSource::Browser { browser } => {
            if lower.contains("could not find")
                && (lower.contains("cookie") || lower.contains("profile"))
            {
                return format!(
                    "{} cookie profile was not found. Open the browser once and verify the selected profile.",
                    browser.label()
                );
            }
            if lower.contains("database is locked")
                || lower.contains("could not copy")
                || lower.contains("failed to copy")
            {
                return format!(
                    "{} cookies are in use. Close the browser completely, then try again.",
                    browser.label()
                );
            }
            if lower.contains("keychain")
                || lower.contains("keyring")
                || lower.contains("permission denied")
                || lower.contains("operation not permitted")
                || lower.contains("access denied")
            {
                return format!(
                    "Access to {} cookies was denied by the OS. Grant terminal/keychain permission and try again.",
                    browser.label()
                );
            }
            if lower.contains("decrypt") {
                return format!(
                    "{} cookies could not be decrypted. Unlock the OS keychain or use a Netscape cookie file.",
                    browser.label()
                );
            }
            stderr.to_string()
        }
        CookieSource::File { path } => {
            if lower.contains("no such file")
                || lower.contains("not found")
                || lower.contains("permission denied")
            {
                return "The configured cookie file was not found or is unreadable. Check the path and file permissions.".to_string();
            }
            stderr.replace(&path.to_string_lossy().to_string(), "<cookie-file>")
        }
        CookieSource::None => stderr.to_string(),
    }
}

impl Config {
    pub fn config_dir() -> Result<PathBuf> {
        let dirs =
            ProjectDirs::from("com", "dloor", "dloor-tui").ok_or(Error::ConfigDirUnavailable)?;
        Ok(dirs.config_dir().to_path_buf())
    }

    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    pub fn log_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("dloor.log"))
    }

    pub fn history_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("history.jsonl"))
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
            cookies: CookieSource::Browser {
                browser: Browser::Firefox,
            },
        };

        config.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), config);
    }

    #[test]
    fn old_config_without_browser_still_loads() {
        let config: Config = toml::from_str(
            r#"
default_quality = "Best"

[destination]
type = "local"
path = "/tmp"
"#,
        )
        .unwrap();

        assert_eq!(config.cookies, CookieSource::None);
    }

    #[test]
    fn old_browser_setting_migrates_to_cookie_source() {
        let config: Config = toml::from_str(
            r#"
default_quality = "Best"
browser = "firefox"

[destination]
type = "local"
path = "/tmp"
"#,
        )
        .unwrap();

        assert_eq!(
            config.cookies,
            CookieSource::Browser {
                browser: Browser::Firefox
            }
        );
        let serialized = toml::to_string(&config).unwrap();
        assert!(serialized.contains("[cookies]"));
        assert!(!serialized.contains("browser = \"firefox\"\n\n[destination]"));
    }

    #[test]
    fn cookie_arguments_cover_browser_file_and_disabled_sources() {
        assert!(CookieSource::None.yt_dlp_args().is_empty());
        assert_eq!(
            CookieSource::Browser {
                browser: Browser::Firefox
            }
            .yt_dlp_args(),
            ["--cookies-from-browser", "firefox"]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            CookieSource::File {
                path: PathBuf::from("/private/cookies.txt")
            }
            .yt_dlp_args(),
            ["--cookies", "/private/cookies.txt"]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn cookie_file_path_is_redacted_from_process_errors() {
        let source = CookieSource::File {
            path: PathBuf::from("/private/cookies.txt"),
        };
        let message = sanitized_ytdlp_error("failed reading /private/cookies.txt", &source);

        assert!(!message.contains("/private/cookies.txt"));
        assert!(message.contains("<cookie-file>"));
    }
}

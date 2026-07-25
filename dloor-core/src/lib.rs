//! Core download API for dloor.
//!
//! This crate intentionally has no TUI dependencies. Frontends create a
//! [`DownloadRequest`], run a [`DownloadJob`], and subscribe to
//! [`DownloadEvent`] values over a channel.

pub mod config;
pub mod deps;
pub mod download;
pub mod platform;

pub use config::{Browser, Config, Destination};
pub use deps::{check_dependencies, DependencyReport, Tool};
pub use download::{
    parse_progress_line, DownloadEvent, DownloadFailure, DownloadItem, DownloadJob,
    DownloadProgress, DownloadRequest, DownloadSuccess, DownloadSummary, Format, PlaylistSelection,
    Quality,
};
pub use platform::{detect_platform, Platform};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported URL: {0}")]
    UnsupportedUrl(String),
    #[error("configuration directory is unavailable on this OS")]
    ConfigDirUnavailable,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML decode error: {0}")]
    TomlDecode(#[from] toml::de::Error),
    #[error("TOML encode error: {0}")]
    TomlEncode(#[from] toml::ser::Error),
    #[error("missing required tool: {0}")]
    MissingTool(&'static str),
    #[error("process failed: {0}")]
    ProcessFailed(String),
    #[error("download cancelled")]
    Cancelled,
    #[error("download produced no output file")]
    MissingOutputFile,
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

use std::{fmt, path::PathBuf};

use clap::{Parser, ValueEnum};
use dloor_core::{
    detect_platform, Config, Destination, DownloadFailure, DownloadSuccess, DownloadSummary,
    DownloadWarning, DownloadWarningKind, Format, Platform, PlaylistSelection, Quality,
};
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "dloor-cli",
    version,
    about = "Non-interactive multimedia downloads through dloor-core"
)]
pub struct Cli {
    /// One or more authorized media URLs to process in order.
    #[arg(value_name = "URL", required = true, num_args = 1..)]
    pub urls: Vec<String>,

    /// Output media format.
    #[arg(long, value_enum, default_value = "video")]
    pub format: CliFormat,

    /// Output quality. Defaults to default_quality from config.toml.
    #[arg(long, value_enum)]
    pub quality: Option<CliQuality>,

    /// Local directory where completed files are saved.
    #[arg(long, value_name = "DIR", required = true)]
    pub output: PathBuf,

    /// Expand and download every item in each playlist URL.
    #[arg(long)]
    pub playlist: bool,

    /// Permit valid URLs on hosts that dloor does not explicitly recognize.
    #[arg(long)]
    pub allow_generic: bool,

    /// Read settings from this config.toml instead of dloor's standard path.
    #[arg(long, value_name = "FILE", conflicts_with = "no_config")]
    pub config: Option<PathBuf>,

    /// Ignore saved settings and use dloor's safe defaults.
    #[arg(long, conflicts_with = "config")]
    pub no_config: bool,

    /// Emit one machine-readable JSON document on stdout.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum CliFormat {
    Audio,
    #[default]
    Video,
}

impl From<CliFormat> for Format {
    fn from(value: CliFormat) -> Self {
        match value {
            CliFormat::Audio => Self::Audio,
            CliFormat::Video => Self::Video,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CliQuality {
    Best,
    Compressed,
    #[value(name = "720p")]
    P720,
    #[value(name = "1080p")]
    P1080,
    #[value(name = "1440p")]
    P1440,
    #[value(name = "2160p")]
    P2160,
}

impl From<CliQuality> for Quality {
    fn from(value: CliQuality) -> Self {
        match value {
            CliQuality::Best => Self::Best,
            CliQuality::Compressed => Self::Compressed,
            CliQuality::P720 => Self::P720,
            CliQuality::P1080 => Self::P1080,
            CliQuality::P1440 => Self::P1440,
            CliQuality::P2160 => Self::P2160,
        }
    }
}

pub struct ResolvedOptions {
    pub config: Config,
    pub format: Format,
    pub quality: Quality,
    pub playlist: PlaylistSelection,
}

#[derive(Clone)]
pub struct ValidatedUrl {
    pub value: String,
    pub platform: Platform,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    InvalidUrl { position: usize },
    GenericUrlRequiresOptIn { position: usize },
    AudioResolution,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl { position } => {
                write!(formatter, "URL {position} is not a valid HTTP or HTTPS URL")
            }
            Self::GenericUrlRequiresOptIn { position } => write!(
                formatter,
                "URL {position} uses an unverified host; pass --allow-generic to proceed"
            ),
            Self::AudioResolution => {
                formatter.write_str("resolution quality values cannot be used with --format audio")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

pub fn load_config(cli: &Cli) -> dloor_core::Result<Config> {
    if cli.no_config {
        Ok(Config::default())
    } else if let Some(path) = &cli.config {
        Config::load_from(path)
    } else {
        Config::load_or_default()
    }
}

pub fn resolve_options(cli: &Cli, mut config: Config) -> Result<ResolvedOptions, ValidationError> {
    let format = Format::from(cli.format);
    let explicitly_selected_quality = cli.quality.map(Quality::from);
    if format == Format::Audio
        && explicitly_selected_quality.is_some_and(|quality| quality.height().is_some())
    {
        return Err(ValidationError::AudioResolution);
    }

    let quality = match (format, explicitly_selected_quality) {
        (_, Some(quality)) => quality,
        (Format::Audio, None) if config.default_quality.height().is_some() => Quality::Best,
        (_, None) => config.default_quality,
    };
    config.destination = Destination::Local {
        path: cli.output.clone(),
    };

    Ok(ResolvedOptions {
        config,
        format,
        quality,
        playlist: if cli.playlist {
            PlaylistSelection::All
        } else {
            PlaylistSelection::Single
        },
    })
}

pub fn validate_urls(
    urls: &[String],
    allow_generic: bool,
) -> Result<Vec<ValidatedUrl>, ValidationError> {
    urls.iter()
        .enumerate()
        .map(|(index, raw)| {
            let position = index + 1;
            let value = raw.trim();
            let platform =
                detect_platform(value).map_err(|_| ValidationError::InvalidUrl { position })?;
            if platform == Platform::Generic && !allow_generic {
                return Err(ValidationError::GenericUrlRequiresOptIn { position });
            }
            Ok(ValidatedUrl {
                value: value.to_string(),
                platform,
            })
        })
        .collect()
}

pub const fn progress_enabled(
    stdout_is_terminal: bool,
    stderr_is_terminal: bool,
    json: bool,
) -> bool {
    stdout_is_terminal && stderr_is_terminal && !json
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UrlStatus {
    Succeeded,
    PartiallySucceeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Serialize)]
pub struct SuccessRecord {
    pub index: usize,
    pub playlist_index: Option<usize>,
    pub title: String,
    pub path: String,
}

impl From<DownloadSuccess> for SuccessRecord {
    fn from(value: DownloadSuccess) -> Self {
        Self {
            index: value.item.index,
            playlist_index: value.item.playlist_index,
            title: value.item.title,
            path: value.path,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct FailureRecord {
    pub index: usize,
    pub playlist_index: Option<usize>,
    pub title: String,
    pub error: String,
}

impl From<DownloadFailure> for FailureRecord {
    fn from(value: DownloadFailure) -> Self {
        Self {
            index: value.item.index,
            playlist_index: value.item.playlist_index,
            title: value.item.title,
            error: value.error,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct WarningRecord {
    pub index: usize,
    pub playlist_index: Option<usize>,
    pub title: String,
    pub kind: &'static str,
    pub message: String,
}

impl From<DownloadWarning> for WarningRecord {
    fn from(value: DownloadWarning) -> Self {
        Self {
            index: value.item.index,
            playlist_index: value.item.playlist_index,
            title: value.item.title,
            kind: warning_kind_name(value.kind),
            message: value.message,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct UrlResult {
    pub url: String,
    pub status: UrlStatus,
    pub total: usize,
    pub succeeded: Vec<SuccessRecord>,
    pub failed: Vec<FailureRecord>,
    pub warnings: Vec<WarningRecord>,
    pub error: Option<String>,
}

impl UrlResult {
    pub fn from_summary(url: String, summary: DownloadSummary, cancelled: bool) -> Self {
        let status = if cancelled {
            UrlStatus::Cancelled
        } else {
            match (summary.succeeded.is_empty(), summary.failed.is_empty()) {
                (false, true) => UrlStatus::Succeeded,
                (false, false) => UrlStatus::PartiallySucceeded,
                (true, _) => UrlStatus::Failed,
            }
        };
        Self {
            url,
            status,
            total: summary.total,
            succeeded: summary
                .succeeded
                .into_iter()
                .map(SuccessRecord::from)
                .collect(),
            failed: summary
                .failed
                .into_iter()
                .map(FailureRecord::from)
                .collect(),
            warnings: summary
                .warnings
                .into_iter()
                .map(WarningRecord::from)
                .collect(),
            error: None,
        }
    }

    pub fn failed(url: String, error: String) -> Self {
        Self {
            url,
            status: UrlStatus::Failed,
            total: 0,
            succeeded: Vec::new(),
            failed: Vec::new(),
            warnings: Vec::new(),
            error: Some(error),
        }
    }

    pub fn is_success(&self) -> bool {
        self.status == UrlStatus::Succeeded
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Totals {
    pub urls: usize,
    pub items: usize,
    pub succeeded: usize,
    pub failed: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonReport {
    pub schema_version: u8,
    pub success: bool,
    pub results: Vec<UrlResult>,
    pub totals: Totals,
    pub error: Option<String>,
}

impl JsonReport {
    pub fn from_results(results: Vec<UrlResult>) -> Self {
        let totals = Totals {
            urls: results.len(),
            items: results.iter().map(|result| result.total).sum(),
            succeeded: results.iter().map(|result| result.succeeded.len()).sum(),
            failed: results.iter().map(|result| result.failed.len()).sum(),
        };
        let success = results.iter().all(UrlResult::is_success);
        Self {
            schema_version: 1,
            success,
            results,
            totals,
            error: None,
        }
    }

    pub fn fatal(error: impl Into<String>) -> Self {
        Self {
            schema_version: 1,
            success: false,
            results: Vec::new(),
            totals: Totals::default(),
            error: Some(error.into()),
        }
    }
}

pub const fn warning_kind_name(kind: DownloadWarningKind) -> &'static str {
    match kind {
        DownloadWarningKind::SubtitleEmbedding => "subtitle_embedding",
        DownloadWarningKind::ThumbnailEmbedding => "thumbnail_embedding",
        DownloadWarningKind::ChapterEmbedding => "chapter_embedding",
        DownloadWarningKind::SubtitleSidecar => "subtitle_sidecar",
        DownloadWarningKind::OptionalPostProcessing => "optional_post_processing",
        DownloadWarningKind::ResolutionFallback => "resolution_fallback",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use dloor_core::DownloadItem;

    fn parse(arguments: &[&str]) -> Cli {
        Cli::try_parse_from(arguments).unwrap()
    }

    #[test]
    fn parses_multiple_urls_and_defaults() {
        let cli = parse(&[
            "dloor-cli",
            "--output",
            "/tmp/downloads",
            "https://youtube.com/watch?v=one",
            "https://youtube.com/watch?v=two",
        ]);

        assert_eq!(cli.urls.len(), 2);
        assert_eq!(cli.format, CliFormat::Video);
        assert_eq!(cli.quality, None);
        assert!(!cli.playlist);
        assert!(!cli.json);
    }

    #[test]
    fn parses_format_quality_playlist_and_json() {
        let cli = parse(&[
            "dloor-cli",
            "--output",
            "/tmp/downloads",
            "--format",
            "video",
            "--quality",
            "1080p",
            "--playlist",
            "--json",
            "https://youtube.com/playlist?list=one",
        ]);

        assert_eq!(cli.format, CliFormat::Video);
        assert_eq!(cli.quality, Some(CliQuality::P1080));
        assert!(cli.playlist);
        assert!(cli.json);
    }

    #[test]
    fn requires_output_and_at_least_one_url() {
        assert!(Cli::try_parse_from(["dloor-cli", "https://youtube.com/watch?v=one"]).is_err());
        assert!(Cli::try_parse_from(["dloor-cli", "--output", "/tmp"]).is_err());
    }

    #[test]
    fn config_and_no_config_conflict() {
        assert!(Cli::try_parse_from([
            "dloor-cli",
            "--output",
            "/tmp",
            "--config",
            "custom.toml",
            "--no-config",
            "https://youtube.com/watch?v=one",
        ])
        .is_err());
    }

    #[test]
    fn explicit_audio_resolution_is_rejected() {
        let cli = parse(&[
            "dloor-cli",
            "--output",
            "/tmp",
            "--format",
            "audio",
            "--quality",
            "720p",
            "https://youtube.com/watch?v=one",
        ]);

        assert!(matches!(
            resolve_options(&cli, Config::default()),
            Err(ValidationError::AudioResolution)
        ));
    }

    #[test]
    fn configured_resolution_falls_back_to_best_for_audio() {
        let cli = parse(&[
            "dloor-cli",
            "--output",
            "/tmp",
            "--format",
            "audio",
            "https://youtube.com/watch?v=one",
        ]);
        let config = Config {
            default_quality: Quality::P2160,
            ..Config::default()
        };

        let resolved = resolve_options(&cli, config).unwrap();

        assert_eq!(resolved.quality, Quality::Best);
        assert_eq!(resolved.format, Format::Audio);
    }

    #[test]
    fn explicit_quality_overrides_config_and_output_is_always_local() {
        let cli = parse(&[
            "dloor-cli",
            "--output",
            "/tmp/downloads",
            "--quality",
            "compressed",
            "https://youtube.com/watch?v=one",
        ]);
        let config = Config {
            default_quality: Quality::P720,
            destination: Destination::Cloud {
                remote: "remote".to_string(),
                path: "videos".to_string(),
            },
            ..Config::default()
        };

        let resolved = resolve_options(&cli, config).unwrap();

        assert_eq!(resolved.quality, Quality::Compressed);
        assert_eq!(
            resolved.config.destination,
            Destination::Local {
                path: PathBuf::from("/tmp/downloads")
            }
        );
    }

    #[test]
    fn generic_urls_require_explicit_opt_in() {
        let urls = vec!["https://example.com/video".to_string()];

        assert_eq!(
            validate_urls(&urls, false).err().unwrap(),
            ValidationError::GenericUrlRequiresOptIn { position: 1 }
        );
        assert_eq!(
            validate_urls(&urls, true).unwrap()[0].platform,
            Platform::Generic
        );
    }

    #[test]
    fn invalid_urls_are_rejected_without_echoing_the_value() {
        let raw = "not a URL";
        let error = validate_urls(&[raw.to_string()], true).err().unwrap();

        assert_eq!(error, ValidationError::InvalidUrl { position: 1 });
        assert!(!error.to_string().contains(raw));
    }

    #[test]
    fn progress_requires_both_terminals_and_human_output() {
        assert!(progress_enabled(true, true, false));
        assert!(!progress_enabled(false, true, false));
        assert!(!progress_enabled(true, false, false));
        assert!(!progress_enabled(true, true, true));
    }

    #[test]
    fn partial_download_summary_makes_the_report_fail() {
        let success_item = DownloadItem {
            index: 1,
            total: 2,
            title: "first".to_string(),
            playlist_index: Some(1),
        };
        let failed_item = DownloadItem {
            index: 2,
            total: 2,
            title: "second".to_string(),
            playlist_index: Some(2),
        };
        let summary = DownloadSummary {
            total: 2,
            succeeded: vec![DownloadSuccess {
                item: success_item,
                path: "/tmp/first.mp4".to_string(),
            }],
            failed: vec![DownloadFailure {
                item: failed_item,
                error: "upstream failure".to_string(),
            }],
            warnings: Vec::new(),
        };

        let report = JsonReport::from_results(vec![UrlResult::from_summary(
            "url".to_string(),
            summary,
            false,
        )]);

        assert!(!report.success);
        assert_eq!(report.results[0].status, UrlStatus::PartiallySucceeded);
        assert_eq!(report.totals.succeeded, 1);
        assert_eq!(report.totals.failed, 1);
    }

    #[test]
    fn warnings_do_not_change_a_successful_exit_result() {
        let item = DownloadItem {
            index: 1,
            total: 1,
            title: "item".to_string(),
            playlist_index: None,
        };
        let summary = DownloadSummary {
            total: 1,
            succeeded: vec![DownloadSuccess {
                item: item.clone(),
                path: "/tmp/item.mp4".to_string(),
            }],
            failed: Vec::new(),
            warnings: vec![DownloadWarning {
                item,
                kind: DownloadWarningKind::ResolutionFallback,
                message: "used a lower resolution".to_string(),
            }],
        };

        let report = JsonReport::from_results(vec![UrlResult::from_summary(
            "url".to_string(),
            summary,
            false,
        )]);

        assert!(report.success);
        assert_eq!(report.results[0].status, UrlStatus::Succeeded);
        assert_eq!(report.results[0].warnings.len(), 1);
    }
}

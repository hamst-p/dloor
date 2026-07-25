use std::{
    path::{Path, PathBuf},
    process::Command as StdCommand,
};

use time::{Date, Duration, Month, OffsetDateTime};
use tokio::{process::Command as TokioCommand, sync::mpsc};

use crate::{config::Destination, download::DownloadEvent, Config, MediaOptions};

pub const YT_DLP_STALE_DAYS: i64 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    YtDlp,
    Ffmpeg,
    Rclone,
}

impl Tool {
    pub fn command(self) -> &'static str {
        match self {
            Self::YtDlp => "yt-dlp",
            Self::Ffmpeg => "ffmpeg",
            Self::Rclone => "rclone",
        }
    }

    pub fn install_hint(self) -> &'static str {
        match self {
            Self::YtDlp => "Install yt-dlp: https://github.com/yt-dlp/yt-dlp#installation",
            Self::Ffmpeg => "Install ffmpeg: https://ffmpeg.org/download.html",
            Self::Rclone => "Install rclone and run `rclone config`: https://rclone.org/install/",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YtDlpVersion {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl YtDlpVersion {
    fn date(self) -> Option<Date> {
        Date::from_calendar_date(self.year, Month::try_from(self.month).ok()?, self.day).ok()
    }

    pub fn is_older_than_90_days_on(self, today: Date) -> bool {
        self.date()
            .is_some_and(|released| today - released > Duration::days(YT_DLP_STALE_DAYS))
    }
}

impl std::fmt::Display for YtDlpVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{:04}.{:02}.{:02}",
            self.year, self.month, self.day
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FfmpegVersion {
    pub major: u32,
    pub minor: Option<u32>,
    pub patch: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParsedToolVersion {
    YtDlp(YtDlpVersion),
    Ffmpeg(FfmpegVersion),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolVersion {
    pub tool: Tool,
    pub raw: String,
    pub parsed: Option<ParsedToolVersion>,
    pub executable: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionFreshness {
    Current,
    OlderThan90Days,
    Unknown,
}

#[derive(Debug, Clone, Default)]
pub struct DependencyReport {
    pub missing_required: Vec<Tool>,
    pub missing_optional: Vec<Tool>,
    pub versions: Vec<ToolVersion>,
    pub warnings: Vec<String>,
}

impl DependencyReport {
    pub fn is_ready(&self) -> bool {
        self.missing_required.is_empty()
    }

    pub fn version(&self, tool: Tool) -> Option<&ToolVersion> {
        self.versions.iter().find(|version| version.tool == tool)
    }

    pub fn yt_dlp_freshness_on(&self, today: Date) -> VersionFreshness {
        match self.version(Tool::YtDlp).and_then(|version| version.parsed) {
            Some(ParsedToolVersion::YtDlp(version)) if version.is_older_than_90_days_on(today) => {
                VersionFreshness::OlderThan90Days
            }
            Some(ParsedToolVersion::YtDlp(_)) => VersionFreshness::Current,
            Some(ParsedToolVersion::Ffmpeg(_)) | None => VersionFreshness::Unknown,
        }
    }

    pub fn message(&self) -> String {
        let mut lines = Vec::new();
        if !self.missing_required.is_empty() {
            lines.push("Missing required tools:".to_string());
            lines.extend(
                self.missing_required
                    .iter()
                    .map(|tool| format!("- {}: {}", tool.command(), tool.install_hint())),
            );
        }
        if !self.missing_optional.is_empty() {
            lines.push("Optional cloud upload tools are unavailable:".to_string());
            lines.extend(
                self.missing_optional
                    .iter()
                    .map(|tool| format!("- {}: {}", tool.command(), tool.install_hint())),
            );
        }
        lines.join("\n")
    }
}

pub fn check_dependency_presence(config: Option<&Config>) -> DependencyReport {
    let mut report = DependencyReport::default();
    for tool in [Tool::YtDlp, Tool::Ffmpeg] {
        if which::which(tool.command()).is_err() {
            report.missing_required.push(tool);
        }
    }

    let needs_rclone = config
        .map(|config| matches!(config.destination, Destination::Cloud { .. }))
        .unwrap_or(false);
    if needs_rclone && which::which(Tool::Rclone.command()).is_err() {
        report.missing_required.push(Tool::Rclone);
    } else if !needs_rclone && which::which(Tool::Rclone.command()).is_err() {
        report.missing_optional.push(Tool::Rclone);
    }

    report
}

pub fn check_dependencies(config: Option<&Config>) -> DependencyReport {
    let mut report = check_dependency_presence(config);
    for tool in [Tool::YtDlp, Tool::Ffmpeg] {
        if !report.missing_required.contains(&tool) {
            if let Some(version) = capture_tool_version(tool) {
                report.versions.push(version);
            }
        }
    }
    if report.yt_dlp_freshness_on(OffsetDateTime::now_utc().date())
        == VersionFreshness::OlderThan90Days
    {
        if let Some(version) = report.version(Tool::YtDlp) {
            report.warnings.push(format!(
                "yt-dlp {} is more than {YT_DLP_STALE_DAYS} days old; run /update before troubleshooting extractor failures.",
                version.raw
            ));
        }
    }
    report
}

fn capture_tool_version(tool: Tool) -> Option<ToolVersion> {
    let executable = which::which(tool.command()).ok()?;
    let arguments: &[&str] = match tool {
        Tool::YtDlp => &["--version"],
        Tool::Ffmpeg => &["-version"],
        Tool::Rclone => return None,
    };
    let output = StdCommand::new(&executable).args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = if stdout.trim().is_empty() {
        stderr.trim()
    } else {
        stdout.trim()
    };
    let raw = combined.lines().next()?.trim().to_string();
    let parsed = match tool {
        Tool::YtDlp => parse_yt_dlp_version(&raw).map(ParsedToolVersion::YtDlp),
        Tool::Ffmpeg => parse_ffmpeg_version(&raw).map(ParsedToolVersion::Ffmpeg),
        Tool::Rclone => None,
    };
    Some(ToolVersion {
        tool,
        raw,
        parsed,
        executable,
    })
}

pub fn parse_yt_dlp_version(raw: &str) -> Option<YtDlpVersion> {
    raw.split_whitespace().find_map(|token| {
        let token = token.trim_start_matches(['v', 'V']);
        let mut parts = token.split('.');
        let year = parts.next()?.parse::<i32>().ok()?;
        let month = parts.next()?.parse::<u8>().ok()?;
        let day = parts.next()?.parse::<u8>().ok()?;
        let version = YtDlpVersion { year, month, day };
        version.date().map(|_| version)
    })
}

pub fn parse_ffmpeg_version(raw: &str) -> Option<FfmpegVersion> {
    let token = raw
        .strip_prefix("ffmpeg version ")?
        .split_whitespace()
        .next()?
        .trim_start_matches(['n', 'N', 'v', 'V']);
    let mut parts = token.split('.');
    let major = numeric_prefix(parts.next()?)?;
    let minor = parts.next().and_then(numeric_prefix);
    let patch = parts.next().and_then(numeric_prefix);
    Some(FfmpegVersion {
        major,
        minor,
        patch,
    })
}

fn numeric_prefix(value: &str) -> Option<u32> {
    let digits = value
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    (!digits.is_empty()).then(|| digits.parse().ok()).flatten()
}

#[derive(Debug, Clone)]
pub struct DependencyJob {
    config: Config,
}

impl DependencyJob {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn spawn(self) -> mpsc::Receiver<DownloadEvent> {
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let fallback_config = self.config.clone();
            let report =
                tokio::task::spawn_blocking(move || check_dependencies(Some(&self.config)))
                    .await
                    .unwrap_or_else(|_| {
                        let mut report = check_dependency_presence(Some(&fallback_config));
                        report
                            .warnings
                            .push("Dependency versions could not be inspected.".to_string());
                        report
                    });
            tx.send(DownloadEvent::DependenciesChecked { report })
                .await
                .ok();
        });
        rx
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YtDlpUpdateOutcome {
    pub success: bool,
    pub output: String,
    pub update_hint: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct YtDlpUpdateJob;

impl YtDlpUpdateJob {
    pub fn spawn(self) -> mpsc::Receiver<DownloadEvent> {
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let executable = which::which(Tool::YtDlp.command()).ok();
            let outcome = run_ytdlp_update(executable.as_deref()).await;
            tx.send(DownloadEvent::YtDlpUpdateFinished { outcome })
                .await
                .ok();
        });
        rx
    }
}

async fn run_ytdlp_update(executable: Option<&Path>) -> YtDlpUpdateOutcome {
    let Some(executable) = executable else {
        return YtDlpUpdateOutcome {
            success: false,
            output: "yt-dlp is not installed or is not available on PATH.".to_string(),
            update_hint: Some(Tool::YtDlp.install_hint().to_string()),
        };
    };
    let mut command = TokioCommand::new(executable);
    command.arg("-U").kill_on_drop(true);
    match command.output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = [stdout.trim(), stderr.trim()]
                .into_iter()
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            let message = if combined.is_empty() {
                format!("yt-dlp -U exited with {}", output.status)
            } else {
                combined
            };
            YtDlpUpdateOutcome {
                success: output.status.success(),
                update_hint: (!output.status.success()).then(|| update_hint(executable, &message)),
                output: message,
            }
        }
        Err(error) => YtDlpUpdateOutcome {
            success: false,
            output: format!("Could not start yt-dlp -U: {error}"),
            update_hint: Some(update_hint(executable, "")),
        },
    }
}

pub fn update_hint(executable: &Path, output: &str) -> String {
    let path = executable.to_string_lossy().to_ascii_lowercase();
    let output = output.to_ascii_lowercase();
    if path.contains("homebrew") || path.contains("/cellar/") || output.contains("homebrew") {
        "This installation appears to be managed by Homebrew. Run: brew upgrade yt-dlp".to_string()
    } else if path.contains("pipx") || output.contains("pipx") {
        "This installation appears to be managed by pipx. Run: pipx upgrade yt-dlp".to_string()
    } else if output.contains("installed with pip")
        || output.contains("installed yt-dlp with pip")
        || output.contains("site-packages")
        || path.contains("/venv/")
    {
        "This installation appears to be managed by pip. Re-run its install command, for example: python -m pip install -U \"yt-dlp[default]\""
            .to_string()
    } else if path.starts_with("/usr/bin/") || path.starts_with("/bin/") {
        "This installation appears to be managed by the OS. Update yt-dlp with the system package manager (for Debian/Ubuntu: sudo apt update && sudo apt install --only-upgrade yt-dlp)."
            .to_string()
    } else {
        "Self-update is unavailable for this installation. Use the original package manager or see https://github.com/yt-dlp/yt-dlp/wiki/Installation"
            .to_string()
    }
}

pub fn check_media_capabilities(options: &MediaOptions) -> Vec<String> {
    if !options.embed_subtitles && !options.embed_thumbnail && !options.embed_chapters {
        return Vec::new();
    }
    let encoders = StdCommand::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned());
    let muxers = StdCommand::new("ffmpeg")
        .args(["-hide_banner", "-muxers"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned());

    match (encoders, muxers) {
        (Some(encoders), Some(muxers)) => {
            media_capability_warnings(options, &encoders, &muxers)
        }
        _ => vec![
            "Optional ffmpeg embedding capabilities could not be inspected; downloads can continue, but embedding may produce warnings."
                .to_string(),
        ],
    }
}

fn media_capability_warnings(options: &MediaOptions, encoders: &str, muxers: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    if options.embed_subtitles && !has_capability(encoders, "mov_text") {
        warnings.push(
            "ffmpeg lacks the mov_text encoder required for MP4 subtitle embedding.".to_string(),
        );
    }
    if (options.embed_thumbnail || options.embed_chapters) && !has_capability(muxers, "mov") {
        warnings.push(
            "ffmpeg lacks the MOV/MP4 muxer required for some thumbnail or chapter embedding."
                .to_string(),
        );
    }
    if (options.embed_thumbnail || options.embed_chapters) && !has_capability(muxers, "mp3") {
        warnings.push(
            "ffmpeg lacks the MP3 muxer required for some audio metadata embedding.".to_string(),
        );
    }
    warnings
}

fn has_capability(output: &str, name: &str) -> bool {
    output.lines().any(|line| {
        line.split_whitespace()
            .any(|field| field.split(',').any(|candidate| candidate == name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stable_and_nightly_yt_dlp_versions() {
        assert_eq!(
            parse_yt_dlp_version("2026.06.09"),
            Some(YtDlpVersion {
                year: 2026,
                month: 6,
                day: 9
            })
        );
        assert_eq!(
            parse_yt_dlp_version("2026.06.09.232829"),
            Some(YtDlpVersion {
                year: 2026,
                month: 6,
                day: 9
            })
        );
        assert_eq!(parse_yt_dlp_version("unknown"), None);
    }

    #[test]
    fn parses_common_ffmpeg_version_lines() {
        assert_eq!(
            parse_ffmpeg_version("ffmpeg version 7.1.1 Copyright"),
            Some(FfmpegVersion {
                major: 7,
                minor: Some(1),
                patch: Some(1)
            })
        );
        assert_eq!(
            parse_ffmpeg_version("ffmpeg version n6.0-static"),
            Some(FfmpegVersion {
                major: 6,
                minor: Some(0),
                patch: None
            })
        );
    }

    #[test]
    fn ninety_day_boundary_matches_upstream_policy() {
        let version = YtDlpVersion {
            year: 2026,
            month: 1,
            day: 1,
        };
        let day_90 = Date::from_calendar_date(2026, Month::April, 1).unwrap();
        let day_91 = Date::from_calendar_date(2026, Month::April, 2).unwrap();

        assert!(!version.is_older_than_90_days_on(day_90));
        assert!(version.is_older_than_90_days_on(day_91));
    }

    #[test]
    fn update_hints_follow_the_detected_installation() {
        assert!(update_hint(Path::new("/opt/homebrew/bin/yt-dlp"), "").contains("brew upgrade"));
        assert!(update_hint(
            Path::new("/home/me/.local/pipx/venvs/yt-dlp/bin/yt-dlp"),
            ""
        )
        .contains("pipx upgrade"));
        assert!(update_hint(Path::new("/usr/bin/yt-dlp"), "").contains("system package manager"));
        assert!(update_hint(
            Path::new("/home/me/.local/bin/yt-dlp"),
            "ERROR: You installed yt-dlp with pip"
        )
        .contains("pip install"));
    }

    #[test]
    fn optional_media_capabilities_are_reported_without_blocking() {
        let options = MediaOptions {
            embed_subtitles: true,
            embed_thumbnail: true,
            embed_chapters: true,
            ..MediaOptions::default()
        };
        let warnings = media_capability_warnings(
            &options,
            " S..... mov_text 3GPP Timed Text subtitle",
            " E mov,mp4,m4a QuickTime / MOV\n E mp3 MP3",
        );
        assert!(warnings.is_empty());

        let warnings = media_capability_warnings(&options, "", "");
        assert_eq!(warnings.len(), 3);
    }
}

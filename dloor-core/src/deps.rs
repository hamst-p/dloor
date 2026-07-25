use std::process::Command;

use crate::{config::Destination, Config, MediaOptions};

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

#[derive(Debug, Clone, Default)]
pub struct DependencyReport {
    pub missing_required: Vec<Tool>,
    pub missing_optional: Vec<Tool>,
}

impl DependencyReport {
    pub fn is_ready(&self) -> bool {
        self.missing_required.is_empty()
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

pub fn check_dependencies(config: Option<&Config>) -> DependencyReport {
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

pub fn check_media_capabilities(options: &MediaOptions) -> Vec<String> {
    if !options.embed_subtitles && !options.embed_thumbnail && !options.embed_chapters {
        return Vec::new();
    }
    let encoders = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned());
    let muxers = Command::new("ffmpeg")
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

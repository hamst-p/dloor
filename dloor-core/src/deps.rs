use crate::{config::Destination, Config};

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

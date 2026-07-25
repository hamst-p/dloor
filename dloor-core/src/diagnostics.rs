use crate::VersionFreshness;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCause {
    AuthenticationRequired,
    GeoRestricted,
    MediaUnavailable,
    FormatUnavailable,
    OutdatedYtDlp,
    NetworkUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorDiagnosis {
    pub cause: ErrorCause,
    pub summary: String,
    pub advice: String,
}

pub fn diagnose_ytdlp_error(
    stderr: &str,
    yt_dlp_version: Option<&str>,
    freshness: VersionFreshness,
) -> Option<ErrorDiagnosis> {
    let normalized = stderr.to_ascii_lowercase();

    if contains_any(
        &normalized,
        &[
            "login required",
            "log in to",
            "sign in to",
            "authentication required",
            "confirm you’re not a bot",
            "confirm you're not a bot",
            "use --cookies",
            "http error 401",
        ],
    ) || (normalized.contains("http error 403")
        && contains_any(
            &normalized,
            &["cookie", "login", "sign in", "authentication"],
        ))
    {
        return Some(ErrorDiagnosis {
            cause: ErrorCause::AuthenticationRequired,
            summary: "This site appears to require authentication.".to_string(),
            advice: "Confirm that you are authorized to access the content, then configure a browser session or cookie file in /settings.".to_string(),
        });
    }

    if contains_any(
        &normalized,
        &[
            "georestrictederror",
            "geo-restricted",
            "geo restricted",
            "not available in your country",
            "not available in your region",
        ],
    ) {
        return Some(ErrorDiagnosis {
            cause: ErrorCause::GeoRestricted,
            summary: "The media appears to be unavailable in the current region.".to_string(),
            advice: "dloor does not bypass regional restrictions. Verify that the content is lawfully available from your location.".to_string(),
        });
    }

    if contains_any(
        &normalized,
        &[
            "has been removed",
            "removed by the uploader",
            "deleted by uploader",
            "this video was deleted",
            "content is no longer available",
        ],
    ) {
        return Some(ErrorDiagnosis {
            cause: ErrorCause::MediaUnavailable,
            summary: "The media appears to have been removed or is no longer available."
                .to_string(),
            advice: "Verify the URL in a browser. Removed media cannot be downloaded by updating settings.".to_string(),
        });
    }

    if contains_any(
        &normalized,
        &[
            "requested format is not available",
            "no video formats found",
            "no audio formats found",
            "format selection failed",
        ],
    ) {
        return Some(ErrorDiagnosis {
            cause: ErrorCause::FormatUnavailable,
            summary: "The requested media format is not available.".to_string(),
            advice: "Return to the format or quality screen and try Best, or refresh metadata after updating yt-dlp.".to_string(),
        });
    }

    let explicit_update = contains_any(
        &normalized,
        &[
            "please update to the latest version",
            "update yt-dlp",
            "your version of yt-dlp is out of date",
        ],
    );
    let extractor_failure = contains_any(
        &normalized,
        &[
            "nsig extraction failed",
            "signature extraction failed",
            "unable to extract",
            "player response",
        ],
    );
    if explicit_update || (extractor_failure && freshness == VersionFreshness::OlderThan90Days) {
        let version = yt_dlp_version.map_or_else(
            || "The installed yt-dlp may be outdated.".to_string(),
            |version| format!("The installed yt-dlp version ({version}) may be outdated."),
        );
        return Some(ErrorDiagnosis {
            cause: ErrorCause::OutdatedYtDlp,
            summary: version,
            advice: "Run /update. If self-update is unavailable, use the package manager that installed yt-dlp.".to_string(),
        });
    }

    if contains_any(
        &normalized,
        &[
            "temporary failure in name resolution",
            "name or service not known",
            "nodename nor servname provided",
            "connection timed out",
            "connection refused",
            "connection reset",
            "network is unreachable",
            "unable to download webpage",
            "ssl: certificate",
            "tls handshake",
        ],
    ) {
        return Some(ErrorDiagnosis {
            cause: ErrorCause::NetworkUnavailable,
            summary: "The network request could not reach the media site.".to_string(),
            advice: "Check DNS, connectivity, proxy settings, and TLS inspection, then retry."
                .to_string(),
        });
    }

    None
}

fn contains_any(value: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| value.contains(marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_upstream_error_shapes() {
        let cases = [
            (
                "ERROR: [youtube] abc: Sign in to confirm you’re not a bot. Use --cookies-from-browser",
                ErrorCause::AuthenticationRequired,
            ),
            (
                "ERROR: This video is not available in your country",
                ErrorCause::GeoRestricted,
            ),
            (
                "ERROR: [youtube] abc: Video has been removed by the uploader",
                ErrorCause::MediaUnavailable,
            ),
            (
                "ERROR: [youtube] Requested format is not available. Use --list-formats",
                ErrorCause::FormatUnavailable,
            ),
            (
                "ERROR: Unable to download webpage: Temporary failure in name resolution",
                ErrorCause::NetworkUnavailable,
            ),
        ];

        for (stderr, expected) in cases {
            assert_eq!(
                diagnose_ytdlp_error(stderr, None, VersionFreshness::Unknown)
                    .unwrap()
                    .cause,
                expected,
                "{stderr}"
            );
        }
    }

    #[test]
    fn extractor_failure_only_suggests_update_when_version_is_old() {
        let stderr = "ERROR: [youtube] nsig extraction failed";
        assert_eq!(
            diagnose_ytdlp_error(
                stderr,
                Some("2025.01.01"),
                VersionFreshness::OlderThan90Days
            )
            .unwrap()
            .cause,
            ErrorCause::OutdatedYtDlp
        );
        assert_eq!(
            diagnose_ytdlp_error(stderr, Some("2026.06.09"), VersionFreshness::Current),
            None
        );
    }

    #[test]
    fn explicit_update_request_does_not_require_parsed_version() {
        assert_eq!(
            diagnose_ytdlp_error(
                "ERROR: Please update to the latest version",
                None,
                VersionFreshness::Unknown
            )
            .unwrap()
            .cause,
            ErrorCause::OutdatedYtDlp
        );
    }

    #[test]
    fn ambiguous_errors_are_not_guessed() {
        for stderr in [
            "ERROR: HTTP Error 403: Forbidden",
            "ERROR: Video unavailable",
            "ERROR: Unsupported URL",
            "ERROR: Something unexpected happened",
        ] {
            assert_eq!(
                diagnose_ytdlp_error(stderr, None, VersionFreshness::Unknown),
                None,
                "{stderr}"
            );
        }
    }
}

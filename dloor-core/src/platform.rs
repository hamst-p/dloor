use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Platform {
    YouTube,
    YouTubeShorts,
    Instagram,
    InstagramReels,
    TikTok,
    Facebook,
    X,
    Generic,
}

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Self::YouTube => "YouTube",
            Self::YouTubeShorts => "YouTube Shorts",
            Self::Instagram => "Instagram",
            Self::InstagramReels => "Instagram Reels",
            Self::TikTok => "TikTok",
            Self::Facebook => "Facebook",
            Self::X => "X (Twitter)",
            Self::Generic => "Generic (yt-dlp)",
        }
    }
}

pub fn detect_platform(url: &str) -> Result<Platform> {
    let trimmed = url.trim();
    let (scheme, remainder) = trimmed
        .split_once("://")
        .ok_or_else(|| Error::UnsupportedUrl(trimmed.to_string()))?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https")
        || trimmed.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(
                    character,
                    '\\' | '`' | '$' | ';' | '|' | '<' | '>' | '"' | '\''
                )
        })
    {
        return Err(Error::UnsupportedUrl(trimmed.to_string()));
    }

    let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    if authority.is_empty() || authority.contains('@') || !valid_authority(authority) {
        return Err(Error::UnsupportedUrl(trimmed.to_string()));
    }

    let authority_lower = authority.to_ascii_lowercase();
    let host = authority_lower
        .trim_start_matches("www.")
        .split(':')
        .next()
        .unwrap_or_default()
        .to_string();
    let path = remainder[authority_end..]
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    match host.as_str() {
        "youtube.com" | "m.youtube.com" | "music.youtube.com" => {
            if path.starts_with("/shorts/") {
                Ok(Platform::YouTubeShorts)
            } else {
                Ok(Platform::YouTube)
            }
        }
        "youtu.be" => Ok(Platform::YouTube),
        "instagram.com" | "m.instagram.com" => {
            if path.starts_with("/reel/") || path.starts_with("/reels/") {
                Ok(Platform::InstagramReels)
            } else {
                Ok(Platform::Instagram)
            }
        }
        "tiktok.com" | "m.tiktok.com" | "vt.tiktok.com" | "vm.tiktok.com" => Ok(Platform::TikTok),
        "facebook.com" | "m.facebook.com" | "fb.watch" => Ok(Platform::Facebook),
        "x.com" | "twitter.com" | "mobile.twitter.com" | "t.co" => Ok(Platform::X),
        _ => Ok(Platform::Generic),
    }
}

fn valid_authority(authority: &str) -> bool {
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()) => {
            (host, Some(port))
        }
        Some(_) => return false,
        None => (authority, None),
    };
    if port.is_some_and(|port| port.parse::<u16>().map_or(true, |value| value == 0)) {
        return false;
    }
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return true;
    }
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
                && label.starts_with(|ch: char| ch.is_ascii_alphanumeric())
                && label.ends_with(|ch: char| ch.is_ascii_alphanumeric())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_platforms() {
        let cases = [
            ("https://www.youtube.com/watch?v=abc", Platform::YouTube),
            ("https://youtube.com/shorts/abc", Platform::YouTubeShorts),
            ("https://www.instagram.com/p/abc/", Platform::Instagram),
            ("https://instagram.com/reel/abc/", Platform::InstagramReels),
            ("https://vt.tiktok.com/ZSNabc/", Platform::TikTok),
            ("https://www.facebook.com/watch/?v=123", Platform::Facebook),
            ("https://fb.watch/abc/", Platform::Facebook),
            ("https://x.com/user/status/123", Platform::X),
            ("https://twitter.com/user/status/123", Platform::X),
        ];

        for (url, expected) in cases {
            assert_eq!(detect_platform(url).unwrap(), expected, "{url}");
        }
    }

    #[test]
    fn unknown_valid_hosts_are_generic() {
        assert_eq!(
            detect_platform("https://example.com/video?id=1").unwrap(),
            Platform::Generic
        );
        assert_eq!(
            detect_platform("http://127.0.0.1:8080/video").unwrap(),
            Platform::Generic
        );
    }

    #[test]
    fn rejects_malformed_or_command_like_inputs() {
        for value in [
            "youtube.com/watch?v=abc",
            "file:///tmp/video",
            "https:///missing-host",
            "https://bad host/video",
            "https://example.com/video;rm",
            "https://user@example.com/video",
            "--exec=command",
            "https://example.com:99999/video",
        ] {
            assert!(detect_platform(value).is_err(), "{value}");
        }
    }
}

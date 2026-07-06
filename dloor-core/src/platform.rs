use regex::Regex;

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
        }
    }
}

pub fn detect_platform(url: &str) -> Result<Platform> {
    let trimmed = url.trim();
    let capture = Regex::new(r"(?i)^(?:https?://)?(?:www\.)?([^/?#]+)([^?#]*)")
        .expect("platform URL regex compiles")
        .captures(trimmed)
        .ok_or_else(|| Error::UnsupportedUrl(trimmed.to_string()))?;

    let host = capture
        .get(1)
        .map(|m| m.as_str().to_ascii_lowercase())
        .unwrap_or_default();
    let path = capture
        .get(2)
        .map(|m| m.as_str().to_ascii_lowercase())
        .unwrap_or_default();

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
        _ => Err(Error::UnsupportedUrl(trimmed.to_string())),
    }
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
    fn rejects_unknown_urls() {
        assert!(detect_platform("https://example.com/video").is_err());
    }
}

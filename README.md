# dloor

[![CI](https://github.com/hamst-p/dloor/actions/workflows/ci.yml/badge.svg)](https://github.com/hamst-p/dloor/actions/workflows/ci.yml)
![release date](https://img.shields.io/github/hamst-p/dloor)

> Educational purpose only.
>
> dloor is intended for learning, personal tooling experiments, and lawful personal archiving. Use it only with content you own, content in the public domain, or content you have explicit permission to download. You are responsible for complying with copyright law, platform terms of service, and any other rules that apply in your jurisdiction. This project does not encourage or support infringement, redistribution of copyrighted material, bypassing access controls, or downloading private content without authorization.

`dloor` is a Rust multimedia downloader powered by `yt-dlp` and `ffmpeg`. It
provides both a Ratatui terminal UI (`dloor`) and a non-interactive,
pipe-friendly command (`dloor-cli`).

For content you are authorized to access, dloor can ask `yt-dlp` to read the
logged-in session from a supported local browser or use a Netscape-format cookie
file. dloor never reads or copies cookie contents itself.

Supported URL families:

- YouTube
- YouTube Shorts
- Instagram
- Instagram Reels
- TikTok
- Facebook
- X / Twitter
- Other valid `http://` or `https://` URLs supported by yt-dlp (unverified;
  confirmation required by default)

Unknown hosts are passed through to yt-dlp only after dloor validates the URL
syntax and displays an “unverified host” confirmation. This does not expand
dloor's access rights or bypass any site's controls.

## Status

dloor is early-stage OSS software. The local download flow is the primary supported path today. Cloud upload support is designed around `rclone`, but should be treated as experimental while the project matures.

## Features

- Download a single authorized media item as video or audio
- Preview title, uploader, duration, and reported resolutions before enqueueing
- Preview a playlist's total count and first five titles without fetching every item
- Expand an authorized playlist and process its items sequentially
- Continue a playlist after individual item failures and report a final summary
- Show labeled current-item and overall progress bars with speed and ETA
- Queue multiple downloads while one job runs in the background
- Reorder, remove, monitor, or cancel queued jobs
- Keep the latest 500 item results in a local JSON Lines history
- Retry a failed playlist item without repeating successful items
- Write selected subtitle sidecars and optionally request subtitle, thumbnail,
  and chapter embedding
- Keep a usable media file when optional embedding fails and report the issue as
  a completion warning
- Select 720p, 1080p, 1440p, or 2160p video bounds when available
- Apply an optional validated yt-dlp bandwidth limit
- Run scripted or batch downloads through the non-interactive `dloor-cli`
- Publish completed local files without overwriting existing names or exposing
  partially copied final files
- Save locally or upload through an optional `rclone` remote

## Quick Start

Install system prerequisites:

```bash
# macOS
brew install rust yt-dlp ffmpeg

# optional, only needed for cloud upload experiments
brew install rclone
```

Clone and run:

```bash
git clone https://github.com/hamst-p/dloor.git
cd dloor
cargo run -p dloor-tui
```

Install the `dloor` command locally:

```bash
cargo install --path dloor-tui --force
cargo install --path dloor-cli --force
dloor
dloor-cli --help
```

## Installation Options

### Prebuilt Binaries

Tagged releases provide archives for:

- macOS on Apple Silicon (`aarch64-apple-darwin`)
- macOS on Intel (`x86_64-apple-darwin`)
- Linux x86_64 with glibc (`x86_64-unknown-linux-gnu`)

Download the archive for your platform from
[GitHub Releases](https://github.com/hamst-p/dloor/releases), verify it against
the accompanying `SHA256SUMS`, extract it, and move `dloor` and/or `dloor-cli`
to a directory on your `PATH`. The archive does not bundle `yt-dlp`, `ffmpeg`,
or `rclone`; install those separately as described under Requirements.

Windows is not a supported release target in this phase. The Rust UI stack is
portable, but process cancellation, updater/package-manager guidance, filesystem
publication semantics, and the external-tool installation path have not yet
been validated together on Windows. Building from source there may work, but it
is not covered by CI or release testing.

### Install From GitHub

If Rust is already installed, you can install directly from the repository:

```bash
cargo install --git https://github.com/hamst-p/dloor.git --package dloor-tui --bin dloor
cargo install --git https://github.com/hamst-p/dloor.git --package dloor-cli --bin dloor-cli
dloor
dloor-cli --help
```

The executables are usually installed under `~/.cargo/bin`. If your shell
cannot find them, make sure that directory is included in your `PATH`.

### Build From Source

```bash
git clone https://github.com/hamst-p/dloor.git
cd dloor
cargo build --release
./target/release/dloor
./target/release/dloor-cli --help
```

For day-to-day development:

```bash
cargo run -p dloor-tui
```

## Requirements

### Rust

Install Rust with `rustup` if you do not already have it:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Then restart your shell or source Cargo's environment file:

```bash
source "$HOME/.cargo/env"
```

### External Tools

dloor shells out to existing media tools instead of bundling them.

Required:

- `yt-dlp`
- `ffmpeg`

Optional:

- `rclone`, only needed for cloud upload experiments

macOS:

```bash
brew install yt-dlp ffmpeg rclone
```

Debian / Ubuntu:

```bash
sudo apt update
sudo apt install ffmpeg rclone
python3 -m pip install -U yt-dlp
```

Arch Linux:

```bash
sudo pacman -S ffmpeg rclone yt-dlp
```

Verify your tools:

```bash
yt-dlp --version
ffmpeg -version
rclone version
```

`rclone` can be missing if you only use local downloads.

dloor reads `yt-dlp --version` and `ffmpeg -version` in the background after
the TUI starts. The results are cached for the current process, so version
inspection does not block the visible startup path. A date-based yt-dlp version
more than 90 days old is shown as a main-screen warning, matching yt-dlp's own
staleness policy.

## First Run

Start the app:

```bash
dloor
```

On first launch, choose where downloads should be saved.

- Local: save files to a local directory
- Cloud: upload files through an `rclone` remote and remote path

Cookie authentication is off by default. Enable a browser session or cookie file
only when a URL requires credentials you are authorized to use.

If you choose cloud storage, configure `rclone` first:

```bash
rclone config
```

Then enter the remote name and remote path in dloor's setup screen.

## Usage

1. Paste a supported URL into the main screen, or review the URL prefilled from
   the clipboard.
2. Press `Enter`.
3. For an unknown host, confirm that you want to try yt-dlp.
4. Wait for the cancellable metadata preview and verify the title, uploader,
   duration, resolutions, and playlist sample.
5. For a playlist, choose whether to download one item or the entire playlist.
6. Choose `Video` or `Audio`.
7. Choose `Best`, `Compressed`, or an available resolution for video.
8. The job is added to the queue and the main screen is ready for another URL.
9. Open `/queue` to monitor current-item and overall progress.
10. Open `/history` to review saved paths, failures, or retry a failed item.

### Non-interactive CLI

`dloor-cli` runs the same `dloor-core` download job without opening the TUI:

```bash
dloor-cli \
  --format video \
  --quality best \
  --output "$HOME/Downloads" \
  "https://www.youtube.com/watch?v=EXAMPLE"
```

Pass more than one URL to process them sequentially, and add `--playlist` to
expand each playlist. Valid URLs on hosts that dloor does not recognize require
the explicit `--allow-generic` flag; the TUI's remembered confirmation setting
does not silently opt a script into unverified hosts.

Successful final paths are written one per line to stdout. Diagnostics,
warnings, and an interactive overall progress bar use stderr; progress is
suppressed unless both streams are terminals, so stdout remains safe to pipe.
`--json` replaces the path lines with one versioned JSON document and suppresses
progress.

The CLI returns `0` only when every item succeeds, `1` for execution or partial
failure, `2` for invalid arguments, and `130` after Ctrl-C has cancelled the
active child process. Individual URL failures do not prevent later URLs from
running.

Common options:

- `--format video|audio`: select the output media type; defaults to `video`
- `--quality best|compressed|720p|1080p|1440p|2160p`: override
  `default_quality`
- `--output <DIR>`: required local destination
- `--playlist`: expand each supplied playlist instead of selecting one item
- `--allow-generic`: explicitly permit a syntactically valid, unverified host
- `--config <FILE>`: use an explicit configuration file
- `--no-config`: ignore saved configuration and use built-in defaults
- `--json`: emit one JSON document with `schema_version: 1`

For example, process multiple authorized URLs and keep stdout machine-readable:

```bash
dloor-cli \
  --no-config \
  --json \
  --output "$HOME/Downloads" \
  "https://www.youtube.com/watch?v=EXAMPLE_1" \
  "https://www.youtube.com/watch?v=EXAMPLE_2"
```

### Local Output Safety

Local downloads and post-processing run in an isolated temporary directory.
Only the confirmed output is published to the selected destination. If a file
name already exists, dloor preserves it and appends a numeric suffix such as
`video (1).mp4`.

When the temporary directory and destination are on different filesystems,
dloor copies into a temporary file inside the destination, synchronizes it, and
then publishes it without replacing an existing name. Cancelling before
publication removes temporary work; once publication succeeds, the item is
reported as completed rather than retroactively cancelled. Subtitle sidecars
follow the same publication rules.

## Key Commands

- URL input: paste a URL and press `Enter`
- `/queue`: view, reorder, remove, monitor, or cancel queued jobs
- `/history`: view retained results and retry a selected failed item with `r`
- `/update`: confirm and run `yt-dlp -U`, then show the updater output
- `/settings`: open destination settings
- `/howtouse`: show the short usage guide
- `/quit`: quit
- `q`: quit from the URL input or completion screen
- `Esc`: go back, or quit from the URL input screen
- `Esc` while metadata is loading: cancel the preview request
- `Esc` during a download: cancel the active `yt-dlp`, `ffmpeg`, or `rclone` process
- Error screen `↑` / `↓`, `PageUp` / `PageDown`, `Home` / `End`: scroll long diagnostics
- Error screen `c`: copy the diagnosis, suggested action, and complete raw error
- Arrow keys: move between scope, format, and quality choices
- Queue screen `Ctrl+Up` / `Ctrl+Down`: reorder a waiting job
- Queue screen `c`: cancel the selected job
- Queue screen `d`: remove the selected waiting job without adding a history record
- Queue screen `Enter`: monitor the selected job
- `Tab`: move between setup fields
- `Enter`: confirm

When unfinished jobs exist, a normal quit asks for confirmation. Confirming cancels
the active process and discards waiting jobs before the application exits.

## Quality Presets

- Video / Best: prefers the best available H.264 video and AAC audio, then
  merges/remuxes to MP4. If a platform such as X omits codec metadata, dloor
  falls back to its best audio/video MP4 instead of rejecting the format.
- Video / Compressed: downloads up to 1080p, then re-encodes with `ffmpeg`
  using the validated `[transcode]` preset; the pre-transcode source file is
  removed after a successful conversion
- Video / 720p, 1080p, 1440p, or 2160p: prefers the highest available H.264
  video stream at or below the selected height and AAC audio, with the same MP4
  fallback for incomplete codec metadata
- Audio / Best: extracts m4a at best available quality
- Audio / Compressed: extracts mp3 with `--audio-quality 5`

For a single-video preview, only standard resolution choices actually reported
by yt-dlp are shown. Playlist previews intentionally inspect only the first five
flat entries, so all four standard choices are shown; each item is resolved
independently. If no stream exists at or below the requested height, yt-dlp
falls back to `best`. Whenever the confirmed output height differs from the
request, the completion screen reports the selected height as a warning.

## Configuration

Configuration is stored as `config.toml` in the operating system's standard config directory, resolved through the `directories` crate.

The TUI and CLI share this file by default, including cookies, media embedding,
bandwidth, default quality, and transcode settings. CLI flags override the
matching setting, and `--output` always selects a local destination so a script
cannot upload merely because the TUI was configured for cloud storage. Use
`--no-config` for deterministic built-in defaults or `--config <FILE>` for an
explicit file. CLI runs do not write the TUI download history.

Typical locations:

- macOS: `~/Library/Application Support/com.dloor.dloor-tui/config.toml`
- Linux: `~/.config/dloor-tui/config.toml` or the path selected by your XDG environment

The `default_quality` value controls the initial selection on each Quality screen:

```toml
default_quality = "Compressed"
```

Existing `"Best"` and `"Compressed"` values remain valid. Resolution defaults
use `"720p"`, `"1080p"`, `"1440p"`, or `"2160p"`. If a configured resolution is
not available on the current single-video preview, the Quality screen selects
Best and explains why.

The active Video / Compressed preset can be adjusted with structured values:

```toml
[transcode]
crf = 28
preset = "fast"
max_width = 1920
audio_bitrate_kbps = 128
```

The defaults above preserve the original output. `crf` must be between 0 and
51, `max_width` must be an even value from 16 through 1920, and
`audio_bitrate_kbps` must be from 8 through 512. `preset` is restricted to the
known x264 values `ultrafast`, `superfast`, `veryfast`, `faster`, `fast`,
`medium`, `slow`, `slower`, `veryslow`, and `placebo`. Arbitrary codec names,
filter expressions, and extra ffmpeg arguments are deliberately not accepted.
Existing configurations that omit `[transcode]`, as well as partially specified
presets, receive the current defaults for every missing value.

An optional `bandwidth_limit` accepts positive byte rates such as `"50K"`,
`"4.2M"`, `"1G"`, or an integer byte count:

```toml
bandwidth_limit = "4.2M"
```

Leave the field empty in `/settings` for no limit. The value is passed to
yt-dlp as `--limit-rate` only for actual downloads, not metadata or playlist
preview requests.

Metadata previews and the first download attempt use the same optional cookie
setting. The narrow YouTube HTTP 403 fallback described below explicitly
disables cookies for its retry.
Playlist previews are limited to the first five entries;
this limit is fixed and does not add a `config.toml` field.

`confirm_generic_urls` defaults to `true`, including when an older config file
does not contain it. Choosing `a` on the unverified-host screen stores `false`
globally for all unknown hosts. Open `/settings` and set `Confirm generic URLs`
to `Always` to restore the prompt.

`clipboard_autofill` also defaults to `true` for existing configuration files.
At startup and whenever the main screen is re-entered, dloor reads clipboard
text in a background task and prefills an empty input only when the entire text
is a valid URL. It never starts a download automatically and never replaces
typed input. A URL removed by the user is not proposed again during that
process. Generic URLs still require the configured unverified-host confirmation.
Set `Clipboard URL detection` to `Off` in `/settings`, or use:

```toml
clipboard_autofill = false
```

Runtime diagnostics are written to `dloor.log` beside `config.toml`. The file is
limited to 5 MiB and one `dloor.log.1` backup is retained. Set `RUST_LOG` when
you need a different log level; dloor logs its own crates at `debug` by default.
Diagnostic logs intentionally omit download URLs, local and cloud paths, browser
profiles, cookie-file paths and contents, command arguments, and raw
external-tool error output.

Download history is stored as `history.jsonl` beside `config.toml`. It contains
source URLs, titles, platforms, selected format and quality, output paths, status,
and an RFC 3339 UTC timestamp. The newest 500 entries are retained. Unlike the
diagnostic log, this file intentionally contains URLs and paths so failed items
can be retried; protect the config directory as personal data. On Unix, dloor
creates the history file with owner-only permissions where supported.

Optional media settings are edited in `/settings` and stored with serde defaults,
so existing configuration files keep all options disabled:

```toml
[media]
write_subtitles = true
embed_subtitles = true
subtitle_languages = ["en", "ja"]
include_auto_subtitles = false
embed_thumbnail = true
embed_chapters = true
```

`write_subtitles` preserves subtitle sidecar files. `embed_subtitles` requests
subtitle embedding for video output and implies subtitle download internally;
audio output keeps sidecars instead. An empty language list leaves language
selection to yt-dlp. Automatic subtitles are requested only when subtitle
writing or embedding is enabled.

Optional embedding uses yt-dlp/ffmpeg post-processing. A subtitle, thumbnail, or
chapter embedding failure does not discard a successfully downloaded non-empty
media file; it is shown as a warning on the completion screen. Chapter embedding
is skipped for compressed MP3 output, and thumbnail embedding is skipped before
the custom compressed-video transcode, because those combinations cannot be
retained reliably. Explicit subtitle sidecars are still preserved.

You can reopen the setup screen from inside the app with:

```text
/settings
```

### Cookie Authentication

Open `/settings` and set `Cookies` to `Browser`, `File`, or `Do not use`.
Supported browsers are Chrome, Firefox, Safari, Edge, Brave, Chromium, Vivaldi,
and Opera. File mode accepts a Netscape-format cookie file path.

dloor passes the selection to `yt-dlp --cookies-from-browser` or
`yt-dlp --cookies`; it does not open or parse the cookie file. The browser
profile must be accessible to the current OS user. On macOS, the terminal may
ask for Keychain access. Some Chromium-based browsers may need to be fully
closed while cookies are read. Cookie-file paths and cookie contents are not
written to dloor's diagnostic log or download history.

Use this only for content you own or are explicitly authorized to download. This feature does not bypass DRM, paywalls, account permissions, or other access controls.

## Troubleshooting

### `dloor: command not found`

Make sure Cargo's binary directory is in your `PATH`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

Add that line to your shell profile, such as `~/.zshrc` or `~/.bashrc`, if needed.

### Missing `yt-dlp` or `ffmpeg`

dloor checks for required tools on startup. Install the missing tool, then run `dloor` again.
When optional embedding is enabled, dloor also inspects ffmpeg's reported
encoders and muxers. Missing `mov_text`, MOV/MP4, or MP3 capabilities are
reported as non-blocking warnings; install a full ffmpeg build to enable the
corresponding embedding path.

### Cloud Upload Is Unavailable

Cloud upload requires `rclone`. Install it and run:

```bash
rclone config
```

Then open `/settings` in dloor and enter your remote name.

### Platform Download Fails

The error screen recognizes conservative signatures for authentication, regional
availability, removed media, unavailable formats, an old yt-dlp, and network
failures. A matching summary and suggested action appear above the unmodified
tool error. Ambiguous messages are left unclassified rather than guessed. Press
`c` to copy the complete diagnostic package for a bug report.

Some platforms change frequently, require login, or block automated downloads. Update `yt-dlp` first:

```bash
yt-dlp -U
```

For YouTube and YouTube Shorts, if a cookie-backed media request fails with HTTP
403, dloor retries that item once with cookies explicitly disabled. This can
recover public media when the site's cookie-backed delivery path is temporarily
rejected. A successful retry is reported as a completion warning; content that
actually requires the account still fails normally.

If that does not work, verify that the URL is public and that you have permission to download it.
For an unknown host, “Generic (yt-dlp)” means the URL passed dloor's safety and
syntax checks; it is not a compatibility guarantee.

### Clipboard URL Detection Is Unavailable

Clipboard access is optional. Headless sessions, SSH, and some Wayland
compositors may not expose a readable text clipboard; dloor silently continues
with an empty URL field. Paste manually, or disable `Clipboard URL detection` in
`/settings`. dloor does not log clipboard contents or access failures.

### yt-dlp Is Reported as Old

The warning appears when the date encoded in `yt-dlp --version` is more than 90
days old. Run `/update` and confirm to try `yt-dlp -U`. Self-update works for
official release binaries, but package-managed installations must normally be
updated by the same manager that installed them. If self-update fails, dloor
suggests a command for Homebrew, pipx, pip, or the operating-system package
manager when it can identify one. dloor never updates automatically.

### Cookie Authentication Fails

- Confirm that you are signed in to the selected site in the selected browser.
- Try closing the browser before starting the download.
- Allow terminal access if macOS prompts for Keychain permission.
- In file mode, confirm the file exists, is readable, and uses Netscape cookie
  format.
- Run `yt-dlp --cookies-from-browser chrome "URL"` directly to inspect the full upstream error, replacing `chrome` with your selected browser.
- Disable cookie authentication again in `/settings` when downloading public content.

## Development

Run checks before opening a pull request:

```bash
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

Pull requests and pushes to `main` run the same checks on Ubuntu through GitHub
Actions, with Cargo registry, Git, and build caches. Pushing a `v*` tag whose
value exactly matches the shared Cargo package version runs the quality gate,
builds the three supported native targets, publishes `.tar.gz` archives, and
generates `SHA256SUMS` in a GitHub Release.

Project layout:

- `dloor-core`: UI-independent core library for platform detection, configuration, dependency checks, and download jobs
- `dloor-tui`: Ratatui / crossterm terminal frontend
- `dloor-cli`: non-interactive, pipe-friendly frontend using the same core event stream
- `.github/workflows`: pull-request checks and tagged native-release automation

The core download logic is intentionally separated from the TUI. Future chat integrations, such as Telegram or Slack agents, can reuse `dloor-core` by subscribing to the same `DownloadEvent` stream.

## Contributing

Issues and pull requests are welcome. Please keep changes focused, run the checks above, and avoid adding code paths that bypass platform terms, access controls, or privacy boundaries.

## License

Licensed under the MIT License. See [LICENSE](LICENSE) for details.

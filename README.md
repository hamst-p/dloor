# dloor

> Educational purpose only.
>
> dloor is intended for learning, personal tooling experiments, and lawful personal archiving. Use it only with content you own, content in the public domain, or content you have explicit permission to download. You are responsible for complying with copyright law, platform terms of service, and any other rules that apply in your jurisdiction. This project does not encourage or support infringement, redistribution of copyrighted material, bypassing access controls, or downloading private content without authorization.

`dloor` is a Rust TUI multimedia downloader powered by `yt-dlp` and `ffmpeg`. It detects supported social/video URLs automatically and lets you download media as video or audio from a terminal UI.

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

## Status

dloor is early-stage OSS software. The local download flow is the primary supported path today. Cloud upload support is designed around `rclone`, but should be treated as experimental while the project matures.

## Features

- Download a single authorized media item as video or audio
- Preview title, uploader, duration, and reported resolutions before enqueueing
- Preview a playlist's total count and first five titles without fetching every item
- Expand an authorized playlist and process its items sequentially
- Continue a playlist after individual item failures and report a final summary
- Show separate current-item and overall playlist progress
- Queue multiple downloads while one job runs in the background
- Reorder, remove, monitor, or cancel queued jobs
- Keep the latest 500 item results in a local JSON Lines history
- Retry a failed playlist item without repeating successful items
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
dloor
```

## Installation Options

### Install From GitHub

If Rust is already installed, you can install directly from the repository:

```bash
cargo install --git https://github.com/hamst-p/dloor.git --package dloor-tui --bin dloor
dloor
```

The executable is usually installed at `~/.cargo/bin/dloor`. If your shell cannot find `dloor`, make sure `~/.cargo/bin` is included in your `PATH`.

### Build From Source

```bash
git clone https://github.com/hamst-p/dloor.git
cd dloor
cargo build --release
./target/release/dloor
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

1. Paste a supported URL into the main screen.
2. Press `Enter`.
3. Wait for the cancellable metadata preview and verify the title, uploader,
   duration, resolutions, and playlist sample.
4. For a playlist, choose whether to download one item or the entire playlist.
5. Choose `Video` or `Audio`.
6. Choose `Best` or `Compressed`.
7. The job is added to the queue and the main screen is ready for another URL.
8. Open `/queue` to monitor current-item and overall progress.
9. Open `/history` to review saved paths, failures, or retry a failed item.

## Key Commands

- URL input: paste a URL and press `Enter`
- `/queue`: view, reorder, remove, monitor, or cancel queued jobs
- `/history`: view retained results and retry a selected failed item with `r`
- `/settings`: open destination settings
- `/howtouse`: show the short usage guide
- `/quit`: quit
- `q`: quit from the URL input or completion screen
- `Esc`: go back, or quit from the URL input screen
- `Esc` while metadata is loading: cancel the preview request
- `Esc` during a download: cancel the active `yt-dlp`, `ffmpeg`, or `rclone` process
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

- Video / Best: downloads `bestvideo*+bestaudio/best` and merges to mp4
- Video / Compressed: downloads up to 1080p, then re-encodes with `ffmpeg` using `libx264 -crf 28 -preset fast`; the pre-transcode source file is removed after a successful conversion
- Audio / Best: extracts m4a at best available quality
- Audio / Compressed: extracts mp3 with `--audio-quality 5`

## Configuration

Configuration is stored as `config.toml` in the operating system's standard config directory, resolved through the `directories` crate.

Typical locations:

- macOS: `~/Library/Application Support/com.dloor.dloor-tui/config.toml`
- Linux: `~/.config/dloor-tui/config.toml` or the path selected by your XDG environment

The `default_quality` value controls the initial selection on each Quality screen:

```toml
default_quality = "Compressed"
```

Metadata previews use the same optional cookie setting as the eventual download.
Playlist previews are limited to the first five entries;
this limit is fixed and does not add a `config.toml` field.

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

### Cloud Upload Is Unavailable

Cloud upload requires `rclone`. Install it and run:

```bash
rclone config
```

Then open `/settings` in dloor and enter your remote name.

### Platform Download Fails

Some platforms change frequently, require login, or block automated downloads. Update `yt-dlp` first:

```bash
yt-dlp -U
```

If that does not work, verify that the URL is public and that you have permission to download it.

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
cargo fmt --all
cargo test
cargo clippy -- -D warnings
```

Project layout:

- `dloor-core`: UI-independent core library for platform detection, configuration, dependency checks, and download jobs
- `dloor-tui`: Ratatui / crossterm terminal frontend

The core download logic is intentionally separated from the TUI. Future chat integrations, such as Telegram or Slack agents, can reuse `dloor-core` by subscribing to the same `DownloadEvent` stream.

## Contributing

Issues and pull requests are welcome. Please keep changes focused, run the checks above, and avoid adding code paths that bypass platform terms, access controls, or privacy boundaries.

## License

Licensed under the MIT License. See [LICENSE](LICENSE) for details.

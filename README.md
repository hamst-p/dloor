# dloor

> Educational purpose only.
>
> dloor is intended for learning, personal tooling experiments, and lawful personal archiving. Use it only with content you own, content in the public domain, or content you have explicit permission to download. You are responsible for complying with copyright law, platform terms of service, and any other rules that apply in your jurisdiction. This project does not encourage or support infringement, redistribution of copyrighted material, bypassing access controls, or downloading private content without authorization.

`dloor` is a Rust TUI multimedia downloader powered by `yt-dlp` and `ffmpeg`. It detects supported social/video URLs automatically and lets you download media as video or audio.

Supported URL families:

- YouTube
- YouTube Shorts
- Instagram
- Instagram Reels
- TikTok
- Facebook
- X / Twitter

## Project Structure

- `dloor-core`: UI-independent core library for platform detection, configuration, dependency checks, and download jobs
- `dloor-tui`: Ratatui / crossterm terminal frontend

The core download logic is intentionally separated from the TUI. Future chat integrations, such as Telegram or Slack agents, can reuse `dloor-core` by subscribing to the same `DownloadEvent` stream.

## Requirements

Required:

- `yt-dlp`
- `ffmpeg`

Optional for cloud uploads:

- `rclone`
- A preconfigured remote from `rclone config`

macOS example:

```bash
brew install yt-dlp ffmpeg rclone
```

Linux example:

```bash
python3 -m pip install -U yt-dlp
sudo apt install ffmpeg rclone
```

## Build And Run

For development:

```bash
cargo build --release
cargo run -p dloor-tui
```

To install the `dloor` command:

```bash
cargo install --path dloor-tui --force
dloor
```

The executable is usually installed at `~/.cargo/bin/dloor`. If your shell cannot find `dloor`, make sure `~/.cargo/bin` is included in your `PATH`.

## Tests

```bash
cargo test
cargo clippy -- -D warnings
```

## Usage

On first launch, choose where downloads should be saved.

- Local: save files to a local directory
- Cloud: upload files through an `rclone` remote and remote path

On the main screen, paste a URL and press `Enter`. Then choose the output format and quality. The download screen shows progress in real time.

## Key Commands

- URL input: paste a URL and press `Enter`
- `/settings`: open destination settings
- `/howtouse`: show the short usage guide
- `/quit`: quit
- `q`: quit from the URL input or completion screen
- `Esc`: go back, or quit from the URL input screen
- Arrow keys: move between choices
- `Tab`: move between setup fields
- `Enter`: confirm

## Quality Presets

- Video / Best: downloads `bestvideo*+bestaudio/best` and merges to mp4
- Video / Compressed: downloads up to 1080p, then re-encodes with `ffmpeg` using `libx264 -crf 28 -preset fast`
- Audio / Best: extracts m4a at best available quality
- Audio / Compressed: extracts mp3 with `--audio-quality 5`

## Configuration

Configuration is stored as `config.toml` in the operating system's standard config directory, resolved through the `directories` crate.

# dloor

> Educational purpose only.
>
> dloor is intended for learning, personal tooling experiments, and lawful personal archiving. Use it only with content you own, content in the public domain, or content you have explicit permission to download. You are responsible for complying with copyright law, platform terms of service, and any other rules that apply in your jurisdiction. This project does not encourage or support infringement, redistribution of copyrighted material, bypassing access controls, or downloading private content without authorization.
>
> このプロジェクトは教育・学習目的のサンプルです。自分が権利を持つコンテンツ、パブリックドメインのコンテンツ、または明示的に許可を得たコンテンツにのみ使用してください。著作権法、各プラットフォームの利用規約、居住地域の法令を遵守する責任は利用者にあります。

`dloor-tui` は `yt-dlp` と `ffmpeg` を利用する Rust 製 TUI マルチメディアダウンローダーです。YouTube、YouTube Shorts、Instagram、Instagram Reels、TikTok、Facebook、X (Twitter) の URL を自動判別し、動画または音声として保存できます。

## 構成

- `dloor-core`: TUI に依存しないダウンロード用ライブラリクレート
- `dloor-tui`: Ratatui / crossterm で実装した TUI フロントエンド

将来的に Telegram や Slack などのチャット連携を追加する場合も、`dloor-core` の `DownloadEvent` ストリームを購読するだけで同じコア処理を再利用できます。

## 前提ツール

必須:

- `yt-dlp`
- `ffmpeg`

クラウド保存を使う場合:

- `rclone`
- 事前に `rclone config` で remote を設定

macOS 例:

```bash
brew install yt-dlp ffmpeg rclone
```

Linux 例:

```bash
python3 -m pip install -U yt-dlp
sudo apt install ffmpeg rclone
```

## ビルドと実行

```bash
cargo build --release
cargo run -p dloor-tui
```

`dloor` コマンドとしてインストールする場合:

```bash
cargo install --path dloor-tui --force
dloor
```

インストール先は通常 `~/.cargo/bin/dloor` です。`dloor` が見つからない場合は、`~/.cargo/bin` が `PATH` に含まれているか確認してください。

テスト:

```bash
cargo test
cargo clippy -- -D warnings
```

## 使い方

初回起動時に保存先を設定します。

- Local: ローカルディレクトリへ保存
- Cloud: `rclone` remote と remote path を指定してアップロード

メイン画面では URL を貼り付けて Enter を押します。次に保存形式と品質を選ぶとダウンロードが始まります。

## キー操作

- URL入力: URL を貼り付けて `Enter`
- `/settings`: 保存先設定を開く
- `/howtouse`: 使い方を表示
- `/quit`: 終了
- `q`: URL入力画面または完了画面で終了
- `Esc`: 戻る、または終了
- 矢印キー: 選択肢の移動
- `Tab`: 設定項目の移動
- `Enter`: 決定

## 品質プリセット

- Video / Best: `bestvideo*+bestaudio/best` を mp4 にマージ
- Video / Compressed: 1080p 上限で取得し、`ffmpeg` で `libx264 -crf 28 -preset fast` に再エンコード
- Audio / Best: m4a、最高品質
- Audio / Compressed: mp3、`--audio-quality 5`

## 設定ファイル

設定は OS 標準の設定ディレクトリに `config.toml` として保存されます。パスは `directories` クレートの `ProjectDirs` に従います。

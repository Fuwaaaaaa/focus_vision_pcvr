# Focus Vision PCVR

VIVE Focus Vision向けオープンソースPCVRストリーミングツール。設定ゼロ、つなぐだけ。

## Features

- **ワイヤレスPCVR** — Wi-Fi経由でSteamVRゲームをFocus Visionにストリーミング
- **設定不要** — 4桁PINを入力するだけで接続
- **低レイテンシー** — NVENC H.265ハードウェアエンコード、RTP+FEC、適応ビットレート
- **コンパニオンアプリ** — ドライバーインストール、PIN表示、HMDへのAPKデプロイをGUIで操作
- **オープンソース** — Virtual Desktopの無料代替

## Quick Start

### PC側

```bash
# コンパニオンアプリをビルド
cargo build --release -p focus-vision-companion

# 起動
./target/release/focus-vision.exe
```

コンパニオンアプリが起動したら:
1. 「Install Driver」でSteamVRドライバーをインストール
2. SteamVRを起動
3. 表示されるPINをメモ

### HMD側

1. Focus VisionをUSBでPCに接続（開発者モードON）
2. コンパニオンアプリの「Deploy」タブでAPKをインストール
3. HMDでアプリを起動し、PINを入力

## Architecture

```
PC (Windows)                          HMD (Focus Vision)
┌─────────────────────┐               ┌──────────────────────┐
│ Companion App (.exe)│               │ OpenXR Client (.apk) │
│ - Driver install    │               │ - PIN entry          │
│ - PIN display       │               │ - Video decode       │
│ - ADB deploy        │               │ - GL rendering       │
└────────┬────────────┘               └──────────┬───────────┘
         │                                       │
┌────────┴────────────┐               ┌──────────┴───────────┐
│ SteamVR Driver      │  Wi-Fi        │ Network Receiver     │
│ - Frame capture     │──────────────→│ - RTP/FEC decode     │
│ - NVENC encode      │  TCP:9944     │ - NAL validation     │
│ - RTP/FEC send      │  UDP:9945     │ - MediaCodec decode  │
│                     │←──────────────│ - Tracking send      │
│ Rust Engine         │  UDP:9947     │                      │
│ - Async pipeline    │               │                      │
└─────────────────────┘               └──────────────────────┘
```

## Build

```bash
# 全体ビルド
./build.bat

# Rust のみ
cargo build --release -p streaming-engine
cargo build --release -p focus-vision-companion

# テスト
cargo test --workspace  # 63 tests
```

## Requirements

- **PC:** Windows 10/11, NVIDIA GPU (GTX 1060+), SteamVR
- **HMD:** VIVE Focus Vision
- **Network:** Wi-Fi 5 (5GHz) 以上推奨

## Project Structure

```
rust/streaming-engine/  — Rust streaming engine (C ABI)
rust/companion-app/     — PC companion GUI app (egui)
rust/common/            — Shared types and constants
driver/                 — C++ OpenVR driver DLL
client/                 — Android OpenXR client
config/                 — TOML configuration
```

## Documentation

- [DESIGN.md](DESIGN.md) — デザインシステム (カラー、フォント、UI)
- [CHANGELOG.md](CHANGELOG.md) — 変更履歴
- [CLAUDE.md](CLAUDE.md) — AI開発ガイドライン
- [TODOS.md](TODOS.md) — 未完了タスク

## License

See [LICENSE](LICENSE).

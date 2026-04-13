<div align="center">

# Focus Vision PCVR

**VIVE Focus Vision向けオープンソースPCVRストリーミングツール**

設定ゼロ、つなぐだけ。

[![License: Dual](https://img.shields.io/badge/License-MIT%20%7C%20Commercial-34D399.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-stable-e8e8ec.svg?logo=rust&logoColor=e8e8ec)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-270%2B-34D399.svg)](#testing)
[![Version](https://img.shields.io/badge/version-2.2.0-34D399.svg)](CHANGELOG.md)

</div>

---

## Features

<table>
<tr>
<td width="50%">

### Streaming
- **ワイヤレスPCVR** — Wi-Fi経由でSteamVRゲームをストリーミング
- **低レイテンシー** — NVENC H.265/H.264ハードウェアエンコード
- **適応ビットレート** — RTP+適応FEC（5-40%）、パケットロスに応じて自動調整
- **96fps対応** — 30〜120fpsまで動的フレームレート
- **フルRGBカラーレンジ** — 色表現の忠実度を向上

</td>
<td width="50%">

### Face Tracking & Foveated
- **Face Tracking** — HTC blendshapes → VRChat OSC（EMAスムージング付き）
- **表情プロファイル** — アバターごとに51ブレンドシェイプ感度を個別調整
- **自動キャリブレーション** — 2ステップガイドでmin/max自動収集
- **Foveated Encoding** — 視線追従で周辺部を圧縮、4プリセット対応

</td>
</tr>
<tr>
<td width="50%">

### UX & ツール
- **設定不要** — 6桁PIN入力だけで接続（TLS 1.3暗号化）
- **コンパニオンアプリ** — ドライバー管理、codec切替、レイテンシーグラフ
- **VR睡眠モード** — 非活動検出で自動省電力、動きで即時復帰
- **HMDダッシュボード** — VR内からビットレート/codec設定を変更

</td>
<td width="50%">

### 品質 & 安全性
- **レイテンシーウォーターフォール** — encode/network/decode/renderの内訳をHMD内表示
- **Protocol v3** — 後方互換ゲート付きプロトコル進化
- **メモリ監視** — プロセスRSS監視、リーク検知（50MB/h閾値）
- **セッションログ** — JSONL記録、7日ローテーション
- **ハプティクスフィードバック** — SteamVR → HMD 完全振動パイプライン

</td>
</tr>
</table>

---

## Quick Start

### PC側（リリースから）

1. [GitHub Releases](../../releases/latest) から `FocusVision-Companion-*.zip` をダウンロード
2. 任意のフォルダに展開
3. `focus-vision.exe` を起動

### PC側（ソースから）

```bash
cargo build --release -p focus-vision-companion
./target/release/focus-vision.exe
```

### セットアップ

```
1. コンパニオンアプリ → 「Install Driver」でSteamVRドライバーをインストール
2. SteamVRを起動 → 表示されるPINをメモ
3. Focus VisionをUSBでPCに接続（開発者モードON）
4. コンパニオンアプリ → 「Deploy」タブでAPKをインストール
5. HMDでアプリを起動 → PINを入力
```

---

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

---

## Build

```bash
# 全体ビルド
./build.bat

# Rust のみ
cargo build --release -p streaming-engine
cargo build --release -p focus-vision-companion

# テスト
cargo test --workspace  # 263+ tests
```

<details>
<summary><b>C++ テスト (GoogleTest)</b></summary>

```bash
cd driver/build && ctest  # QPマップ計算 7 tests
```

</details>

---

## Requirements

| 項目 | 要件 |
|------|------|
| **PC** | Windows 10/11, NVIDIA GPU (GTX 1060+), SteamVR |
| **HMD** | VIVE Focus Vision |
| **Network** | Wi-Fi 5 (5GHz) 以上推奨 |

---

## Project Structure

```
rust/streaming-engine/  — Rust streaming engine (C ABI via cbindgen)
rust/companion-app/     — PC companion GUI app (egui, single .exe)
rust/common/            — Shared types and constants
driver/                 — C++ OpenVR driver DLL
client/                 — Android OpenXR client (Kotlin + C++ NDK)
config/                 — TOML configuration
```

---

## Documentation

| ドキュメント | 内容 |
|-------------|------|
| [ARCHITECTURE.md](ARCHITECTURE.md) | システム構成図・データフロー |
| [DESIGN.md](DESIGN.md) | デザインシステム（カラー、フォント、UI） |
| [SECURITY.md](SECURITY.md) | 脅威モデル・暗号化・PIN認証 |
| [CHANGELOG.md](CHANGELOG.md) | 変更履歴 |
| [CONTRIBUTING.md](CONTRIBUTING.md) | 開発環境セットアップ・貢献ガイド |
| [CLAUDE.md](CLAUDE.md) | AI開発ガイドライン |
| [TODOS.md](TODOS.md) | ロードマップ・未完了タスク |
| [docs/TESTING.md](docs/TESTING.md) | 実機テスト手順書 |
| [docs/E2E_TEST_GUIDE.md](docs/E2E_TEST_GUIDE.md) | E2Eテストガイド |

---

## License

このプロジェクトは**デュアルライセンス**です。

| 用途 | ライセンス | 費用 |
|------|-----------|------|
| 個人利用・教育・研究・非商用OSS | [MIT](LICENSE) | **無料** |
| 商用利用（販売・収益化を含む製品/サービス） | Commercial License | **有料** |

> **商用ライセンスについて:** [GitHub Issue](https://github.com/Fuwaaaaaa/focus_vision_pcvr/issues) からお問い合わせください。

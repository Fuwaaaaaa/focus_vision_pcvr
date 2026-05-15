<div align="center">

# Focus Vision PCVR

**VIVE Focus Vision向けオープンソースPCVRストリーミングツール**

設定ゼロ、つなぐだけ。

[![License: Dual](https://img.shields.io/badge/License-MIT%20%7C%20Commercial-34D399.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-stable-e8e8ec.svg?logo=rust&logoColor=e8e8ec)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-450%2B-34D399.svg)](#testing)
[![Version](https://img.shields.io/badge/version-3.0.0-34D399.svg)](CHANGELOG.md)

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
cargo test --workspace  # 450+ tests

# Companion を「実機なし」で試す（デモモード）
cargo run -p focus-vision-companion -- --demo
```

<details>
<summary><b>C++ テスト (GoogleTest)</b></summary>

```bash
cd driver/build && ctest  # 36 tests (QP map / NVENC ABI / VUI)
```

</details>

---

## 実機なし回帰テスト

VR ヘッドセットを接続しなくても、エンジンの全コアパス
(ストリーミング / 顔追跡 OSC / 触覚 / 睡眠モード / 録画 / 適応 FEC) を
JSON シナリオで end-to-end 検証できます。CI ではすべてのシナリオが
`cargo test --features simulator` で自動実行されます。

```bash
# 全シナリオ + 既存単体/結合テスト (CI と同じコマンド)
cargo test --workspace --features simulator -- --test-threads=1

# シナリオだけ
cargo test -p streaming-engine --features simulator --test scenario_test           -- --test-threads=1
cargo test -p streaming-engine --features simulator --test scenario_transport_test -- --test-threads=1
```

| シナリオ | 検証対象 | 主な assertion |
|---|---|---|
| `golden_path` | TCP/TLS+PIN → 2s ストリーム → 正常切断 | `min_frames_decoded`, `min_video_packets`, `max_connect_duration_ms` |
| `haptic` | `engine::queue_haptic` → TCP `HAPTIC_EVENT (0x38)` → mock-client 受信 | `min_haptic_events_received` |
| `sleep_cycle` | 静止トラッキング → `SleepDetector` 発火 → `SLEEP_ENTER (0x50)` 送信 | `min_sleep_enter_count` |
| `face_tracking` | 51 blendshape → engine OSC bridge → `/avatar/parameters/*` 受信 | `expect_osc_addresses`, `min_osc_messages` |
| `packet_loss` | `UdpSender` で 80% パケットドロップ注入 | `max_video_packets` |
| `recording` | `recording.enabled=true` で `*.h265` ファイル生成 | `expect_recording_files{dir,min_bytes}` |

### 新規シナリオの追加

1. `rust/streaming-engine/tests/scenarios/<name>.json` を作成
   - `Scenario` 構造体 (`src/simulator/scenario.rs`) に従う
   - `deny_unknown_fields` により未知のフィールドはパース時点で失敗
2. テストランナーに `#[test] fn scenario_<name>()` を追加
   - 既存テスト (`tests/scenario_test.rs`) と並走させる場合はそこへ
   - Windows のリソース解放タイミングに敏感な scenario は独立した test binary
     (`tests/scenario_*_test.rs`) に分離するとプロセス分離で安定する
3. `cargo test --features simulator --test scenario_<name>_test` で動作確認

詳細は [`docs/E2E_TEST_GUIDE.md`](docs/E2E_TEST_GUIDE.md) と
[`rust/streaming-engine/src/simulator/scenario.rs`](rust/streaming-engine/src/simulator/scenario.rs)
を参照。

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

# Focus Vision PCVR v3.0.0-rc1

**リリース日:** 2026-05-14
**ステータス:** Release Candidate (実機検証前)

v2.2.1 からの最初の v3 系リリース候補です。コンパニオンアプリの UX 仕上げ、
テスト基盤の強化、リリースインフラ（NSIS インストーラー + Authenticode 署名
パイプライン）を中心に、**実機なしで検証可能なすべての完成度向上**を投入
しています。

実機（VIVE Focus Vision + NVIDIA GPU）での検証は次の Phase 4.x で実施し、
final 版 (v3.0.0) で正式リリースします。

---

## ハイライト

### コンパニオンアプリ
- **Audio / APK / Window の persistence**: 設定したオーディオビットレート、
  APK パス、ウィンドウ位置がアプリ再起動後も保持されます。以前はビットレート
  スライダーを動かしても保存されない不具合がありました。
- **「Reset to defaults」ボタン**: Settings タブの Maintenance グループから、
  全 UI オーバーライドをデフォルトに戻せます。誤クリック防止のため 3 秒の
  2 段階確認付き。
- **録画ディレクトリの inline 検証**: 存在しないパスは黄色の警告、
  ファイル指定は赤エラー、空欄はデフォルトパス利用としてそれぞれ表示。
- **エンジン停止バナー**: `status.json` が 5 秒以上更新されないとき、
  Home タブ上部に赤いバナーで通知します。
- **SteamVR ドライバディレクトリ自動検出**: Windows レジストリ
  （`HKLM\SOFTWARE\Valve\Steam` + WOW6432Node fallback）経由で
  解決するため、SteamVR を非標準パスにインストールしていても
  Install / Uninstall ボタンが動作します。
- **`CompanionApp` のタブモジュール化**: `main.rs` を 1209 LoC から 410 LoC に
  縮減、各タブを独立ファイル化しました。ユーザー体験は不変。

### ストリーミングエンジン
- **オーディオ録画の実行時 toggle (CONFIG_UPDATE 0x05)**: 既存のビデオ録画
  トグル (0x03) と同様、オーディオも HMD 側からエンジン再起動なしで
  ON / OFF を切り替えられます。ビデオとは独立にゲートされるため、
  ビデオを録りつつオーディオだけ停止することが可能。
- **Thermal Governor の engine 配線**: GPU 温度に応じてアダプティブ
  ビットレートの天井を 75 / 85 / 90 °C で段階的に絞る機構。NVML が
  使用可能な環境で `--features nvml` 付きビルドすると有効化されます。
  非 NVIDIA 環境では graceful にスキップ。
- **`status.json` パーサーの抽出**: 旧 `read_engine_status` 内に
  インラインだった JSON 解析を `status_parser` モジュールに分離し、
  egui やファイルシステムなしでテスト可能に。6 ダッシュ PIN 表示の
  軽微な不具合も併せて修正。

### リリースインフラ
- **NSIS Windows インストーラー**: `installer/focus_vision.nsi` から
  `FocusVision-vX.Y.Z-Setup.exe` を生成。CI でビルド・smoke 検証済み。
- **Authenticode 署名パイプライン**: `WINDOWS_PFX_BASE64` /
  `WINDOWS_PFX_PASSWORD` のリポジトリ secret が設定されていれば、
  companion exe とインストーラーの両方に自動署名が走ります。
  証明書未設定時は署名ステップをスキップ（PR・フォーク builds 用）。
- **Android リリース keystore**: `ANDROID_KEYSTORE_BASE64` ベースで
  APK 署名。secret 未設定時は ephemeral keystore を生成する
  fallback パスで `assembleRelease` が成功するため、CI に依存
  しません。
- **Coverage CI (nightly)**: `cargo-llvm-cov` で workspace 全体の
  カバレッジを集計し、Cobertura XML をアーティファクト保存。

### テスト・CI
- **ワークスペーステスト**: 313 → 345（+32）
- **Companion**: 25 → 47（+22）
- **Driver C++ (gtest)**: 13 → 30（+17）
- **CI clippy `-D warnings`** 既存ゲートを維持、coverage ジョブ追加

### ドキュメント
- `docs/SIGNING.md` — 署名手順書
- `docs/USER_GUIDE.md` / `TROUBLESHOOTING.md` / `FAQ.md` — 日本語
  エンドユーザーマニュアル
- `CHANGELOG.md` — v3.0.0-rc1 セクション追加

---

## インストール

### Windows (companion + driver)

1. [Releases](https://github.com/Fuwaaaaaa/focus_vision_pcvr/releases) から
   `FocusVision-v3.0.0-rc1-Setup.exe` をダウンロード
2. 実行（Authenticode 署名のため SmartScreen 警告は出ない想定。
   署名なしビルドの場合は「詳細情報 → 実行」で進行）
3. インストールウィザードに従う
4. SteamVR を起動 → ドライバが自動検出されます

### Android (Focus Vision HMD)

1. companion app の Deploy タブから `Browse...` で APK を選択
2. HMD を USB 接続し ADB デバッグを有効化
3. `Install APK on All Devices` をクリック

詳細は `docs/USER_GUIDE.md` を参照。

---

## Breaking changes

なし（v2.2.1 と完全後方互換）。

## Known Issues / 実機未検証項目

以下は v3.0.0-rc1 時点で**コード実装は完了**しているが、実機検証を
final リリース (v3.0.0) で行う項目です:

- **Dynamic Resolution Scaling (DRS)** — コード未着手、Phase 4.x
- **NVENC VUI フルレンジ FFI** — config は既存、NVENC への配線は未実装
- **NVML 連携（実 GPU 温度）** — コードは完了、ただし NVIDIA GPU
  での閾値妥当性検証は未実施
- **HTC Face Tracking 実カメラ入力** — OSC ブリッジ完成、実機 stream 未検証
- **`engine.rs::run_streaming` の分割と FFI 型統合** — テストネットは
  Pillar 1 で敷いた上で post-rc1 follow-up

詳細は CHANGELOG.md の "Deferred to post-RC1" を参照。

## 検証チェックリスト

実機なしで動作確認できる項目:

```sh
# 1. ビルド
cargo build --release --workspace

# 2. テスト (345 件)
cargo test --workspace

# 3. lint (clippy -D warnings)
cargo clippy --workspace -- -D warnings

# 4. Driver C++ テスト (30 件)
cd driver/build && cmake --build . --config Release && ctest --build-config Release

# 5. companion app の手動 smoke test (3 タブ全部レンダ、Settings 保存可)
cargo run -p focus-vision-companion

# 6. Headless E2E (TCP+TLS+PIN+RTP+FEC+UDP 全部緑)
cargo run -p streaming-engine --bin focus-vision-headless --features simulator
```

## サポート

- Issues: https://github.com/Fuwaaaaaa/focus_vision_pcvr/issues
- ドキュメント: `docs/USER_GUIDE.md`, `docs/FAQ.md`, `docs/TROUBLESHOOTING.md`

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)

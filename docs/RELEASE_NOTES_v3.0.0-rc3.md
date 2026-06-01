# Focus Vision PCVR v3.0.0-rc3

**リリース日:** 2026-05-22
**ステータス:** Release Candidate（実機検証前 / rc2 + 配布物品質強化 + 実機なしエビデンス拡張）
**rc2 からの差分:** 配布物メタデータ / ライセンス開示 / NSIS 前提チェック / Android R8 / clippy clean / シナリオハーネス 5 件追加 / 配布 exe に in-process シミュレーション同梱

v3.0.0-rc2（2026-05-15）の品質強化版です。**新規プロダクト機能はゼロ**。
代わりに「実機なしで人手に渡せる完成品」と言い切れる根拠を積み増しました:
配布物のメタデータ・ライセンス開示、Windows インストーラの前提ランタイム
チェック、Android APK の R8 minify、workspace 全体の clippy clean、そして
ヘッドレスシナリオの大幅拡張（codec 比較・face tracking 波形 4 種・フレーム
ジッタ注入・15 分ロングラン）です。

---

## ハイライト

### 配布物としての完成度

#### Cargo workspace metadata 統一
`[workspace.package]` で `version = "3.0.0-rc3"`, `authors`, `license`,
`repository` を集約し、3 サブクレート（`streaming-engine`,
`focus-vision-companion`, `fvp-common`）で `version.workspace = true` 等を
参照。`scripts/check-versions.ps1` も pre-release 接尾辞と
`workspace.package` 継承を理解するように更新。

#### Third-party ライセンス開示
- `about.toml` で許可ライセンスを列挙（Apache-2.0, MIT, ISC, BSD-2/3-Clause,
  Zlib, OFL-1.1, Ubuntu-font-1.0, MPL-2.0, Unicode-3.0, Unlicense ほか）
- `cargo about generate` で `THIRD_PARTY_LICENSES.html` (565 KB) と
  `THIRD_PARTY_LICENSES.md` (532 KB) を生成し、433 crate 分のライセンス本文を
  全文埋め込み
- NSIS インストーラに HTML 版を同梱（インストール先 = `$INSTDIR\THIRD_PARTY_LICENSES.html`）
- 再生成手順を README に記載

#### NSIS インストーラ: VC++ ランタイム前提チェック
`.onInit` で `HKLM\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64`
を読み、未インストール時は明示的なメッセージ + ダウンロード URL 起動 +
abort。これまではこのチェックがなく、ユーザは SteamVR ドライバロード時に
`MSVCP140.dll` 不在の暗号めいたエラーで詰まる可能性がありました。

#### Android APK: R8 minify + ProGuard rules
`build.gradle.kts` の `release.isMinifyEnabled = true`,
`isShrinkResources = true`、新しい `proguard-rules.pro` で
`NativeActivity` 直下、JNI native methods、MediaCodec callback、Surface、
HTC VR launcher が参照する activity surface を `-keep`。APK サイズと code
load 時間が改善（実測差は CI assembleRelease ログに）。

### コード品質

#### Workspace clippy clean
`cargo clippy --workspace --all-features --all-targets -- -D warnings`
および `rust/streaming-engine/fuzz` の同様コマンド両方で警告ゼロ。
具体的な修正対象（v3.0.0-rc2 時点の rustc 互換でない pre-existing
regressions）:

- `engine.rs`: `(0u64 * tick)` / `(1u64 * tick)` を意図的に残し、
  `#[allow(clippy::erasing_op, clippy::identity_op)]` で説明コメント付き
- `tracking/receiver.rs`: `MutexGuard.clone()` を `*MutexGuard` のコピーに
- `lib.rs`: テスト中の `vec![0u8; N]` を `[0u8; N]` 配列に
- `pipeline.rs` / テスト: 手書き `+sz-1)/sz` を `div_ceil` に
- `control/pairing.rs`: `thread_local` 初期化を `const {}` 化、unneeded
  `return` 削除
- `metrics/session_log.rs`: `>= && <=` を `(2024..=2100).contains(&year)`
- `tests/fuzz_tests.rs` / `fuzz_targets/fuzz_*.rs`: `% 2 == 0` を
  `.is_multiple_of(2)`、`match Ok|Err` を `if let Ok`、`max(1).min(1200)`
  を `.clamp(1, 1200)`

#### CI clippy 強化
`.github/workflows/build.yml` の clippy ステップを `--all-targets` 付きに
拡張し、テストファイルの regression も merge 前に捕捉。

### 配布インストーラに in-process シミュレーションを同梱

これまで「実機なしでフルパイプラインを動かす」には開発者がソースから
`--features simulator` でビルドする必要があり、配布インストーラの
`focus-vision.exe` では `--demo`（見せかけ UI 合成）しか使えなかった。
本リリースから **配布 exe 自体を `--features simulator` 付きでビルド**するように
CI (`companion-build` ジョブ) と `build.bat` を変更。受け取った人が
ヘッドセットなしで、ホームタブの **「▶ Start Simulation」** ボタン /
`focus-vision.exe --simulate` から **実 `StreamingEngine` + モック HMD
クライアント**を起動し、TCP+TLS+PIN+RTP+FEC+UDP+音声 (Opus) のパイプライン
全体を実走できる（`rust/companion-app/src/sim.rs`）。

- 追加される依存は `hound`（WAV I/O）+ `tokio` + `streaming-engine` staticlib
  のみで、テスト専用依存は混入しない
- ドライバ DLL (`driver-build` ジョブ) は引き続き `streaming-engine` を
  `simulator` なしでビルドするため、実 VR ストリーミング経路・バイナリは無影響
- シミュレーションは localhost・OS 割当の空きポートのみ使用し、実エンジン
  稼働中 (`status.json` が新しい) は開始を拒否する

これにより「実機なしで人手に渡せる完成品」は、**開発者の検証だけでなく
エンドユーザーの手元でも**成立する。

### 実機なしエビデンス（シナリオハーネス拡張）

#### Decode latency 計測
`MockClientStats` に `depacketize_latency_us_{p50,p95,p99}` と
`depacketize_samples_count` を追加。`MockClientConfig.measure_decode_latency`
（既定 false）が true のとき、video receiver が「最初の RTP packet → 完成
した frame」までの wall time をサンプルとして蓄積し、ラン完了時に nearest-rank
percentile を集計（最大 20 万サンプルでメモリは ~800 KB に上限）。

#### `scenario_codec_comparison` テスト
新シナリオ `codec_comparison_h264.json` / `codec_comparison_h265.json` を
順次走らせ、両 codec の decode-latency を side-by-side でログ出力。実行例:

```
==================== CODEC COMPARISON ====================
                    H.264                H.265           diff (H265-H264)
decode p50 (us)          77                90               +13
decode p95 (us)         375               748              +373
decode p99 (us)        1089              1539              +450
samples                 193               193
frames                  193               193
packets                1013              1013
==========================================================
```

これは合成 NAL の depacketize までの数値であり、実 MediaCodec のデコード
レイテンシーではありません（実機検証は GA 前タスク）。それでも codec を切り
替えると packetize / FEC shard 数が変わる影響を numeric に観測できるため、
将来の codec 経路 regression を CI ログから検知できます。

#### Face tracking 波形プリセット 4 種追加
`FaceMode` enum に `Blink { hz }`, `Talk { hz }`, `Smile { intensity }`,
`Frown { intensity }` を追加。各プリセットは該当する blendshape インデックス
のみを駆動（例: `Blink` は `EyeLeftBlink` (idx 0) と `EyeRightBlink` (idx 6)
のみを sin で振動、他は 0）。3 つの新シナリオ:

- `face_tracking_patterns.json` (Blink)
- `face_tracking_talk.json` (JawOpen)
- `face_tracking_smile.json` (MouthSmileRight/Left)

OSC 出力を loopback で受信し、想定 blendshape の OSC address が現れる
ことを assertion。

#### Frame jitter injection
新 stimulus `InjectFrameLatency { at_sec, latency_us }` /
`ClearFrameLatency { at_sec }` で synthetic frame producer に追加遅延を
注入。`frame_jitter.json` シナリオは 5 ms ウィンドウと 10 ms ウィンドウを
順次注入、adaptive bitrate / GCC 経路の動作を観測。

#### Long-run stability (15 分) シナリオ + 夜間 CI
`long_run_stability.json` (`duration_sec = 900`) は 30/90/150/240/360/540/720
秒の点で 5〜15% packet loss を交互に注入し、以下を assertion:

- `min_frames_decoded: 30000`
- `min_idr_frames: 5`
- `min_heartbeats_sent: 1500`
- `min_video_packets: 100000`
- `min_decode_latency_samples: 1000`
- `max_decode_latency_us_p99: 200000`

`#[ignore]` で通常 CI からは除外し、`.github/workflows/build.yml` の新ジョブ
`long-run-stability` (windows-latest, 30 分 timeout) が `schedule: cron`
（毎日 03:00 UTC）で実行。PR / push は影響を受けません。

### ドキュメント

- `CLAUDE.md`: 状態タグを "rc3 candidate" に、テスト数を 500+ Rust に更新、
  clippy ゲートを `--all-features --all-targets` に
- `README.md`: バージョンバッジを `3.0.0--rc3` に、Third-party dependencies
  サブセクションを追加し再生成手順を記載
- `docs/USER_GUIDE.md`: VC++ ランタイム要件を Prerequisites テーブルに追加、
  ダウンロード ファイル名を rc3 形式に
- 本リリースノート

---

## Breaking changes

なし。rc2 と完全後方互換。`status.json` schema / TCP control message format /
TOML config field、すべて不変です。

> 例外なし。Android クライアントは rc2 と同じく初回接続時に
> `server_fingerprint.hex` を内部ストレージに作成します（rc2 と同一動作）。

---

## インストール

rc2 と同じ手順、ファイル名のみ差分:

### Windows (companion + driver)
1. [Releases](https://github.com/Fuwaaaaaa/focus_vision_pcvr/releases/tag/v3.0.0-rc3) から
   `FocusVision-3.0.0-rc3-Setup.exe` をダウンロード
2. 実行 → 初回のみ VC++ 2015-2022 Redistributable (x64) のチェックが入る
3. インストールウィザードに従う（SteamVR ドライバ自動登録あり）

### Android (Focus Vision HMD)
1. companion app の Deploy タブから `FocusVision-Client-3.0.0-rc3.apk` を選択
2. HMD を USB 接続し ADB デバッグ有効化
3. `Install APK on All Devices`

詳細は `docs/USER_GUIDE.md`。

---

## Known Issues / 実機未検証項目（rc2 から継続）

実機検証は rc3 → final で実施予定:

- **OpenXR loop / xrWaitFrame 実遅延プロファイル** — `Stimulus::InjectFrameLatency`
  で擬似化済みだが、HMD 実機の本物の swap chain 待ちと差し替えていない
- **MediaCodec H.264 vs H.265 実デコード時間** — シナリオ `codec_comparison_*`
  は depacketize までの synthetic-NAL 経路のみを測定
- **HTC Face Tracking 実カメラ入力 → OSC end-to-end** — 4 プリセット波形で
  経路を駆動済みだが、実カメラ ARKit blendshape は別途検証
- **TLS TOFU 実機検証** — PR #61 の test plan チェックリスト 5 項目
- **SteamVR driver hotload** — vrclient stub なし、実 driver.vrmanifest で要確認
- **NVENC VUI フルレンジ実色彩確認** — ctest 6 件は通過、目視は実機
- **NVML 連携実 GPU 温度** — コード完了、閾値妥当性は実機
- **30 分以上の連続セッション耐久試験** — 15 分の `long_run_stability` は
  CI で回るが、4 時間連続稼働は実機 GPU で別途

---

## 検証チェックリスト（実機なし）

```sh
# 1. ビルド
cargo build --release --workspace
cargo build --release -p streaming-engine --features simulator --bins

# 2. 全テスト (500+ Rust + 36 C++)
cargo test --workspace --features simulator -- --test-threads=1

# 3. 15 分ロングラン（手動）
cargo test --release -p streaming-engine --features simulator \
  --test scenario_transport_test long_run_stability \
  -- --ignored --test-threads=1 --nocapture

# 4. lint (clippy, all features all targets)
cargo clippy --workspace --all-features --all-targets -- -D warnings

# 5. Driver C++ テスト (36 件)
cd driver/build && cmake --build . --config Release \
  && ctest --build-config Release --output-on-failure

# 6. Third-party license 再生成
cargo about generate about.hbs    --output-file THIRD_PARTY_LICENSES.html
cargo about generate about-md.hbs --output-file THIRD_PARTY_LICENSES.md

# 7. Codec 比較ログ (CI 標準実行に含まれる)
cargo test --release -p streaming-engine --features simulator \
  --test scenario_transport_test scenario_codec_comparison \
  -- --test-threads=1 --nocapture

# 8. NSIS installer ローカルビルド
./build.bat

# 9. Android APK + R8 ローカルビルド
cd client && ./gradlew assembleRelease && apksigner verify --print-certs \
  app/build/outputs/apk/release/*.apk

# 10. Companion --demo モード（見せかけ UI 合成）
cargo run -p focus-vision-companion -- --demo

# 11. Companion --simulate モード（実エンジン + モック HMD のフルパイプライン、配布 exe 同梱）
cargo run -p focus-vision-companion --features simulator -- --simulate

# 12. Headless E2E (TCP+TLS+PIN+RTP+FEC+UDP)
cargo run -p streaming-engine --bin focus-vision-headless --features simulator
```

---

## サポート

- Issues: https://github.com/Fuwaaaaaa/focus_vision_pcvr/issues
- ドキュメント: `docs/USER_GUIDE.md`, `docs/FAQ.md`, `docs/TROUBLESHOOTING.md`
- 過去 リリースノート: `docs/RELEASE_NOTES_v3.0.0-rc1.md`, `docs/RELEASE_NOTES_v3.0.0-rc2.md`

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)

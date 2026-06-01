# Focus Vision PCVR v3.0.0

**リリース日:** 2026-06-01
**ステータス:** 一般提供 (General Availability)
**rc3 からの差分:** 配布 exe に in-process シミュレーション同梱 / エンジンの
ライブ `"streaming"` ステータス発行 / シミュレーションのポート衝突修正 /
GA バージョン確定

v3.0.0-rc3（2026-05-22）を正式リリースに昇格したものです。中心テーマは
**「実機（VIVE Focus Vision / SteamVR / NVIDIA GPU / Android 実機）が手元に
なくても、配布物そのもので完成品として動作・検証できる」** ことの完成です。

---

## ハイライト

### 配布インストーラに in-process シミュレーションを同梱

これまで「実機なしでフルパイプラインを動かす」には開発者がソースから
`--features simulator` でビルドする必要があり、配布インストーラの
`focus-vision.exe` では `--demo`（見せかけ UI 合成）しか使えませんでした。

v3.0.0 からは **配布 exe 自体を `--features simulator` 付きでビルド**します
（CI `companion-build` ジョブ + `build.bat`）。受け取った人がヘッドセットなしで、
ホームタブの **「▶ Start Simulation」** ボタン / `focus-vision.exe --simulate`
から**実 `StreamingEngine` + モック HMD クライアント**を起動し、
TCP+TLS+PIN+RTP+FEC+UDP+音声 (Opus) のパイプライン全体を実走できます。
PIN 表示・ライブ統計・サブシステムインジケータがすべて実データで動作します。

- 追加依存は `hound`（WAV I/O）+ `tokio` + `streaming-engine` staticlib のみ。
  テスト専用依存は混入しません。
- ドライバ DLL（`driver-build` ジョブ）は引き続き `streaming-engine` を
  `simulator` なしでビルドするため、実 VR ストリーミング経路・バイナリは無影響。
- シミュレーションは localhost・OS 割当の空きポートのみ使用し、実エンジン
  稼働中は開始を拒否します。

### エンジンのライブ `"streaming"` ステータス発行（重要修正）

コンパニオンの「Connected ＋ライブ統計」表示は `status.json` の `"streaming"`
ペイロード（latency/fps/bitrate/サブシステム）に依存しますが、従来エンジンは
`"waiting"` しか書いておらず、**実機接続時を含めて一度も Connected 画面が
点灯しませんでした**（`--demo` はこの画面を合成で模倣していただけ）。

v3.0.0 ではセッション確立時および約1秒ごとに `"streaming"`（ライブ統計付き）を
発行し、切断時は `"waiting"` に戻します。回帰テスト
`headless_e2e_basic_video_flow` とコンパニオンの `sim_smoke_round_trip` で
契約を固定しています。

### シミュレーションのポート衝突修正

`pick_free_ports` が OS のエフェメラルレンジから基底ポートを取り、派生する
映像/音声受信ポートも同レンジに入っていたため、エンジンのエフェメラル送信
ソケットが解放直後のポートを再利用し、モッククライアントの bind と決定的に
衝突していました（`WSAEADDRINUSE`）。非エフェメラルな連続ブロックを予約する
方式に修正しました。

---

## Breaking changes

なし。rc3 と完全後方互換。`status.json` schema / TCP control message format /
TOML config field、すべて不変です（`"streaming"` ステータスは rc1 以来の
スキーマで定義済みのものを、エンジンが実際に発行するようにしただけです）。

---

## インストール

### Windows (companion + driver)
1. [Releases](https://github.com/Fuwaaaaaa/focus_vision_pcvr/releases/tag/v3.0.0) から
   `FocusVision-3.0.0-Setup.exe` をダウンロード
2. 実行 → 初回のみ VC++ 2015-2022 Redistributable (x64) のチェックが入る
3. インストールウィザードに従う（SteamVR ドライバ自動登録あり）

### 実機なしで試す
- **デモモード（見せかけ UI 合成）:** `focus-vision.exe --demo`
- **シミュレーションモード（実エンジンのフルパイプライン）:** ホームタブの
  **「▶ Start Simulation」** または `focus-vision.exe --simulate`

### Android (Focus Vision HMD)
1. companion app の Deploy タブから `FocusVision-Client-3.0.0.apk` を選択
2. HMD を USB 接続し ADB デバッグ有効化 → `Install APK on All Devices`

詳細は `docs/USER_GUIDE.md`。

---

## Known Issues / 実機未検証項目

以下はコード実装済み・**シミュレーションでは検証済み**ですが、実機（実 GPU /
HMD / カメラ）での最終確認が継続中の項目です。本リリースはプロジェクトの
「実機なしで完成品として配布できる」方針に基づき GA としています。

- **OpenXR loop / xrWaitFrame 実遅延プロファイル** — `InjectFrameLatency` で
  擬似化済みだが、HMD 実機の swap chain 待ちとは差し替えていない
- **MediaCodec H.264 vs H.265 実デコード時間** — `codec_comparison_*` は
  depacketize までの synthetic-NAL 経路のみを測定
- **HTC Face Tracking 実カメラ入力 → OSC end-to-end** — 4 プリセット波形で
  経路を駆動済みだが、実カメラ ARKit blendshape は別途検証
- **TLS TOFU 実機検証** — 実 HMD での fingerprint pinning シナリオ
- **SteamVR driver hotload** — 実 `driver.vrmanifest` ロードの確認
- **NVENC VUI フルレンジ実色彩確認** — ctest は通過、目視は実機
- **NVML 連携実 GPU 温度** — コード完了、閾値妥当性は実機
- **30 分以上の連続セッション耐久試験** — 15 分の `long_run_stability` は
  CI（夜間）で回るが、長時間連続稼働は実機 GPU で別途

---

## サポート

- Issues: https://github.com/Fuwaaaaaa/focus_vision_pcvr/issues
- ドキュメント: `docs/USER_GUIDE.md`, `docs/FAQ.md`, `docs/TROUBLESHOOTING.md`
- 過去リリースノート: `docs/RELEASE_NOTES_v3.0.0-rc1.md`,
  `docs/RELEASE_NOTES_v3.0.0-rc2.md`, `docs/RELEASE_NOTES_v3.0.0-rc3.md`

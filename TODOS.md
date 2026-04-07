# TODOS

## v1.0 スコープ内

### ~~FecEncoderのフレームループ外再利用~~ (完了)
- ReedSolomonインスタンスをFecEncoder内でキャッシュ。shard数が変わらない限り再利用。
- `encode_frame_to_packets_with_fec()`追加、`run_streaming()`で再利用パス使用。

### ~~FECクライアント側RS復元~~ (完了)
- fec_decoder.cppにGF(2^8) Vandermonde行列ベースのReed-Solomon復元を実装。
- reed-solomon-erasure crateと同一の行列構築（V × V_top^(-1)）。

### ~~Timewarpシェーダー型修正~~ (完了)
- sampler2D → samplerExternalOES + GL_OES_EGL_image_external_essl3。
- GL_TEXTURE_2D → GL_TEXTURE_EXTERNAL_OESバインド。

### ~~FecFrameDecoder uint16_t化~~ (完了)
- totalShards/dataShards/shardIndex/receivedCountをuint8_t → uint16_tに変更。
- IDRフレーム（>255シャード）のサイレント破損を防止。

### ~~HeartbeatClient接続 + 適応ビットレート修正~~ (完了)
- HMD側: HeartbeatClient + StatsReporterをOpenXRAppに接続。500ms毎にパケットロス統計をTCPで送信。
- PC側: HEARTBEATメッセージをパースしBandwidthEstimatorにHMD実パケットロスを接続。

### ~~ハートビート + 自動再接続~~ (完了)
- run_streaming()を再接続ループに構造変更。指数バックオフ（1s→16s、max 5回）。
- TCP切断時にセッションcancelトークンで停止→再リッスン。

### ~~エンジン状態IPC~~ (完了)
- ステータスファイル（%APPDATA%/FocusVisionPCVR/status.json）でエンジン↔コンパニオンアプリ通信。
- PIN、接続状態、レイテンシー、FPS、ビットレートを共有。

### ~~Deploy非同期化~~ (完了)
- ADB install/launchを別スレッドに移動。UIフリーズ防止。

### H.265 vs H.264 デコードレイテンシー比較調査
- **What:** Focus Vision実機でMediaCodecのH.265とH.264デコードレイテンシーを計測・比較する
- **Why:** Outside Voiceの指摘: H.264はMediaCodecデコードが2-5ms速い可能性。80Mbpsでは画質差が小さく、レイテンシー目標50msに対して2-5msの差は大きい
- **Context:** config/default.tomlのcodecフィールドで切替可能にし、実測値でどちらを採用するか決定。NVENC側はH.264/H.265両対応が容易
- **Depends on:** Phase 2 (Android側デコード) 実装後に計測可能
- **計測手順（準備済み）:**
  1. `config/default.toml`の`codec = "hevc"`を`"h264"`に変更してPC側を再起動
  2. Focus Visionでストリーミング開始
  3. `adb logcat | grep "decode latency"`で90フレーム毎の平均レイテンシーを取得
  4. 各codecで5分以上計測し、安定後の平均値を比較
  5. `video_decoder.cpp`の`avgDecodeLatencyUs()`で統計取得可能

### ~~Foveated Encoding~~ (完了)
- Eye tracker (OpenXR XR_EXT_eye_gaze_interaction) → TrackingSender経由でgaze座標をPC送信
- NVENCのQP delta map (fovea/mid/periphery 3ゾーン) をピクチャパラメータに接続
- config/default.toml `foveated.enabled = true` で有効化可能

## v1.1 スコープ

### ~~Codec切替UI~~ (完了)
- **What:** コンパニオンアプリにH.264/H.265トグルボタンを追加
- **Why:** config/default.tomlの手動編集なしでcodec切替可能に。レイテンシー比較テストが容易になる
- **Context:** config.rsのcodecフィールド + NVENCのuse_hevcフラグは既に対応済み。UIとconfig書き換えのみ

### ~~リアルタイムレイテンシーグラフ~~ (完了)
- **What:** コンパニオンアプリにsparkline形式のレイテンシー/FPS/パケットロスグラフを追加
- **Why:** 数値だけでは傾向が見えない。スパイクや劣化パターンの視覚化
- **Context:** status.jsonに全データが既にある。egui::plot::Lineで30秒分のリングバッファ描画

### ~~HMD内接続品質オーバーレイ~~ (完了)
- **What:** VR体験中にWi-Fi信号強度/パケットロス率を視野隅に小さく表示
- **Why:** 「なぜカクつくのか」を即座に診断可能に
- **Context:** OpenXR composition layerでクワッドオーバーレイ。StatsReporterのデータをGL描画

### ~~自動Codec選択~~ (完了)
- **What:** 初回接続時にH.264/H.265の両方で短時間ベンチマークし、高速な方を自動選択
- **Why:** ユーザーが手動テスト不要。HMDのMediaCodec実装差を自動吸収
- **Context:** デコードレイテンシー計測（avgDecodeLatencyUs）が既に実装済み。各codec 5秒 × 2回でN=900サンプル
- **Depends on:** Codec切替UIの実装後（codec切替のFFIパスが必要）

### ~~ワンクリックログエクスポート~~ (完了)
- **What:** PC側ログ + HMD logcat + システム情報をzip化して保存するボタン
- **Why:** トラブルシューティング時の「ログを送って」が1クリックに
- **Context:** companion appのADB接続を再利用。IPアドレス等のPIIはサニタイズが必要

## v1.1 準備調査��完了）

### ~~NVENC SDK構造体オフセット検証~~
- **What:** NV_ENC_RC_PARAMSのqpMapModeフィールドオフセットが実際のNVENC SDK v12.2と一致するか検証
- **Why:** インライン構造体のフィールド配置が不正確だとfoveated encoding有効化時にクラッシュまたは無視される
- **Context:** foveatedはデフォル���無効なので現状影響なし。有効化���に実機検証必須

### ~~オーディオパイプラインの仮想オーディオデバイス調査~~ (解決済み)
- EUREKA: WASAPI loopback captureで仮想デバイス不要。カーネルモードドライバーなしでシステム音声をキャプチャ可能。
- 実装済み: `audio/capture.rs` (cpal crate) + `audio/encoder.rs` (Opus)

### ~~Android側Opusデコード + AAudio再生~~ (完了)
- AudioPlayer (audio_player.cpp) のOpusデコード + AAudio低遅延再生を実装済み (c07a19f)。
- libopusをAndroid NDKビルドに統合、AAudioStreamでlow-latency再生。

## v1.2 スコープ

### ~~Face Tracking TCP受信ハンドラ修正~~ (完了)
- FACE_DATA (0x35) のTCPハンドラが未実装だった致命的バグを修正
- OscBridgeへのデータパスを接続、parse_face_data()ヘルパー関数追加

### ~~Face Tracking EMAスムージング~~ (完了)
- blendshapeジッター低減のため指数移動平均フィルタ追加
- [face_tracking]設定セクション（smoothing, osc_port）

### ~~ハプティクスフィードバック~~ (完了)
- SteamVR→PCドライバ→Rust engine→TCP→HMD→xrApplyHapticFeedback
- HAPTIC_EVENT (0x38)プロトコル、fvp_haptic_event() FFI

### ~~タッチセンサー + デッドゾーン~~ (完了)
- trigger_touch, grip_touch, thumbstick_touch, thumbstick_clickポーリング
- HTC VIVE Focus 3コントローラープロファイル追加
- サムスティックデッドゾーン（0.1マグニチュード）

### ~~バッテリーレベル~~ (完了)
- Android sysfs (/sys/class/power_supply/battery/capacity) から実バッテリー読み取り

### ~~VR睡眠モード~~ (完了)
- SleepDetector: ヘッドポーズdeltaで非活動検出、タイムアウト後にビットレート低下
- SLEEP_ENTER/SLEEP_EXIT プロトコル、renderSleepDimming() HMDオーバーレイ
- [sleep_mode]設定セクション

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

## v1.1 準備調査

### ~~オーディオパイプラインの仮想オーディオデバイス調査~~ (解決済み)
- EUREKA: WASAPI loopback captureで仮想デバイス不要。カーネルモードドライバーなしでシステム音声をキャプチャ可能。
- 実装済み: `audio/capture.rs` (cpal crate) + `audio/encoder.rs` (Opus)

### ~~Android側Opusデコード + AAudio再生~~ (完了)
- AudioPlayer (audio_player.cpp) のOpusデコード + AAudio低遅延再生を実装済み (c07a19f)。
- libopusをAndroid NDKビルドに統合、AAudioStreamでlow-latency再生。

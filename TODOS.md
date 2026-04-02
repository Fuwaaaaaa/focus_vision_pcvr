# TODOS

## v1.0 スコープ内

### ~~FecEncoderのフレームループ外再利用~~ (完了)
- ReedSolomonインスタンスをFecEncoder内でキャッシュ。shard数が変わらない限り再利用。
- `encode_frame_to_packets_with_fec()`追加、`run_streaming()`で再利用パス使用。

### H.265 vs H.264 デコードレイテンシー比較調査
- **What:** Focus Vision実機でMediaCodecのH.265とH.264デコードレイテンシーを計測・比較する
- **Why:** Outside Voiceの指摘: H.264はMediaCodecデコードが2-5ms速い可能性。80Mbpsでは画質差が小さく、レイテンシー目標50msに対して2-5msの差は大きい
- **Context:** config/default.tomlのcodecフィールドで切替可能にし、実測値でどちらを採用するか決定。NVENC側はH.264/H.265両対応が容易
- **Depends on:** Phase 2 (Android側デコード) 実装後に計測可能

### ハートビート + 自動再接続の実装
- **What:** ハートビートタイムアウト検出と自動再接続をrun_streamingに実装する
- **Why:** TCP切断検出はあるが、Wi-Fiが半死状態（パケットは届くが不安定）のときに検出できない
- **Context:** 旧HeartbeatMonitor/ReconnectPolicy（デッドコード、a55f0c7で削除）をgit historyから参考にし、engine.rs内に直接実装する方がシンプル。adaptive bitrateは統合済み（4cd0494）
- **Depends on:** なし

## v1.1 準備調査

### ~~オーディオパイプラインの仮想オーディオデバイス調査~~ (解決済み)
- EUREKA: WASAPI loopback captureで仮想デバイス不要。カーネルモードドライバーなしでシステム音声をキャプチャ可能。
- 実装済み: `audio/capture.rs` (cpal crate) + `audio/encoder.rs` (Opus)

### Android側Opusデコード + AAudio再生
- **What:** AudioPlayer (audio_player.cpp) のOpusデコード + AAudio低遅延再生を実装
- **Why:** PC側のオーディオキャプチャ+エンコード+送信は完成。HMD側の受信+再生が未実装
- **Context:** libopusをAndroid NDKビルドに追加、AAudioStreamでlow-latency再生
- **Depends on:** Android NDKビルド環境

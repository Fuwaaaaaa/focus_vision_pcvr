# Changelog

All notable changes to Focus Vision PCVR will be documented in this file.

## [1.1.1] - 2026-04-07

### Fixed
- **TLS制御チャネル修正:** ハンドシェイク後にダミー平文ストリームを返していたバグを修正。制御メッセージが実際のTLS接続上で送受信されるように
- **direct_mode use-after-free修正:** `m_pendingTexture`を生ポインタからComPtrに変更し参照カウント安全性を確保
- **JNI参照リーク修正:** VideoDecoder::init()のエラーパスで`m_javaSurfaceTexture`のグローバル参照を解放
- **FFI unsafe修正:** `fvp_submit_encoded_nal`等のFFI関数に`unsafe`マーキング追加
- **unwrapパニック修正:** trackingポートパース、exportのfile_name()でパニックの可能性を除去
- **Clippy全警告解消:** 33件のclippy警告を修正（map_or→is_some_and、Default derive等）

### Performance
- **NALバッファ clone除去:** `std::mem::take()`で所有権移転。フレーム毎のmemcpy削減（1-5ms/frame）
- **FEC encoder clone除去:** `encode()`が所有権を受け取るように変更。データシャードのコピー削減（2-5ms/frame）
- **レイテンシートラッカー最適化:** `collect()`→`fold()`でVec allocationを除去

### Tests
- **テスト134件に増加**（119→134、+15件）
- TLS handshakeの実際のtokio_rustls接続テスト追加
- tracking パケットパース（gaze拡張、controller）テスト追加
- face tracking OSC全blendshape検証テスト追加
- audio encoder エッジケーステスト追加

## [1.1.0] - 2026-04-06

### Added
- **Codec切替UI:** コンパニオンアプリでH.264/H.265をワンクリック切替。config/local.tomlに保存
- **レイテンシーグラフ:** Homeタブにsparkline形式の30秒レイテンシー/FPSグラフ（egui_plot）
- **ログエクスポート:** PC/HMDログ+システム情報をzip化するワンクリックボタン。PII自動サニタイズ
- **HMD接続品質オーバーレイ:** VR視野にWi-Fi信号強度風の3バーアイコン。パケットロスに応じて緑/黄/赤
- **自動Codec選択:** 初回接続時にH.265/H.264の両方で5秒ベンチマーク→低レイテンシーなcodecを自動選択

## [1.0.0] - 2026-04-06

### Added
- **オーディオストリーミング:** WASAPI loopback → Opus → AAudio。PC音声をHMDで低遅延再生
- **FECクライアント復元:** GF(2^8) Vandermonde行列ベースReed-Solomon。パケットロス耐性
- **Timewarp:** Quaternionベース回転補正。デコード遅延時の頭部追従を維持
- **HeartbeatClient:** 500ms毎にHMD統計（パケットロス、デコードレイテンシー）をTCPで送信
- **適応ビットレート:** HMD実パケットロスをBandwidthEstimatorに接続
- **自動再接続:** 指数バックオフ（1s→16s、max 5回）。TCP切断時にセッション停止→再リッスン
- **エンジン状態IPC:** status.json経由でコンパニオンアプリとPIN/接続状態/統計を共有
- **JNI SurfaceTexture:** zero-copy MediaCodec→GLテクスチャ。ASurfaceTexture_fromSurfaceTexture
- **デコードレイテンシー計測:** submit-to-output wall time。logcatに90フレーム毎の平均出力
- **Android CI:** NDK r26b + Gradle 8.5 + OpenXR SDK FetchContent。APK自動ビルド

### Fixed
- **FecFrameDecoder uint16_t化:** >255シャードのIDRフレームのサイレント破損を防止
- **Timewarpシェーダー型修正:** sampler2D → samplerExternalOES + GL_OES_EGL_image_external_essl3
- **ADB deploy非同期化:** UIフリーズ防止

## [0.1.0.0] - 2026-03-27

### Added
- **PCコンパニオンアプリ:** ドライバーインストール、PIN表示、ADB経由HMDデプロイをGUIで操作。`cargo run -p focus-vision-companion`で起動
- **Real NVENCエンコード:** nvEncodeAPI64.dllをランタイムロード。SDK不要でビルド可能。NVIDIA非搭載環境はテストパターンに自動フォールバック
- **Video pipeline Phase 1 (PC):** NVENCエンコーダー、D3D11テクスチャ入力、DirectMode統合
- **Video pipeline Phase 2 (Android):** MediaCodecデコード (ASurfaceTexture zero-copy出力)、OpenGL ESレンダリング (external OESシェーダー)、UDP受信パイプライン
- **NALバリデーション:** H.265 NALヘッダー検証。不正パケットをドロップしデコーダークラッシュを防止
- **IDRキーフレーム制御:** TCP制御チャンネル経由のIDR_REQUESTメッセージ。E2E: Client→Rust→C++→NvencEncoder
- **新C ABI:** `fvp_submit_encoded_nal()` — C++側でエンコード済みNALデータをRustに渡す
- **`fvp_set_idr_callback()`** — Rust→C++ IDR通知用コールバック登録
- **デザインシステム:** DESIGN.md。Brutally Minimal美学、エメラルドグリーンアクセント、Instrument Serif + Geist + Geist Mono
- **テスト:** IDRフラグ伝搬、NAL→RTPラウンドトリップ、FECリカバリ、TCP制御メッセージ

### Changed
- **SubmittedFrame → EncodedFrame:** Rust側の型をリネーム。nal_data, is_idrフィールド
- **FecEncoder最適化:** ReedSolomonインスタンスをキャッシュ。shard数が同じなら再利用
- **NVENCをC++側に移動:** GPU バッファの跨言語共有を回避 (eng review決定)

### Fixed
- **TCPメッセージ長制限:** 64KB上限追加。悪意あるクライアントのOOM攻撃を防止
- **TCP切断検知:** CancellationToken連携。HMD切断時にストリーミングを停止
- **コールバック安全性:** Cleanup()の順序修正。fvp_shutdown()をs_instance=nullptr前に呼び出し
- **FVPヘッダーエンディアン:** Android側のframe_index/flags読み取りをLEに修正
- **Adversarial review修正 (7件):** FEC shard count計算、最終フレームデコード、整数プロモーション、TCP mid-message cancel、タイムスタンプオーバーフロー、3byte Annex B対応、デッドコード除去

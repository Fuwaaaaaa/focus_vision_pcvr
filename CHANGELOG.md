# Changelog

All notable changes to Focus Vision PCVR will be documented in this file.

## [2.2.1] - 2026-04-15

### Added
- **GccEstimator:** 独立した遅延ベース帯域推定モジュール。DelayTrend状態判定(Normal/Increasing/Overuse)、bitrate_multiplier、プロービング準備
- **BurstDetector:** Wi-Fi干渉(burst) vs 持続的混雑(sustained)の分類。LossPattern enum、500ms閾値でburst→sustained遷移
- **sent_packet_log:** engine.rsにRTP送信タイムスタンプ記録（HashMap<u16, u64>、5000エントリ上限）。GCC推定器の入力
- **congestion_controlトグル:** config.tomlで`congestion_control = "gcc" | "loss"`を選択可能。"loss"モードでは既存ロスベースのみ使用
- **AdaptiveFEC boost:** BurstDetector連携のboost機能（activate/deactivate）、1秒レート制限、effective_redundancy()

### Changed
- **BitrateController:** adjust()がGccEstimatorとBurstDetectorの3引数に拡張。burst時はFEC吸収、sustained時は積極減速
- **BandwidthEstimator:** 遅延計算をGccEstimatorに分離。ロス率EWMAとRTT追跡のみに専念（単一責務）
- **engine.rs:** TRANSPORT_FEEDBACK受信時にGccEstimator.process_feedback()を即時実行（バッチ処理→リアルタイム処理）

### Fixed
- **max reductionバグ:** delay overuse(-10%)とloss(-20%)が累積して-28%になるバグを修正。候補の大きい方のみ採用するmax reduction方式に変更

### Tests
- **テスト296件に増加**（277→296、+19件）
- GccEstimator 7テスト（初期状態、安定リンク、overuse検出、underuse、単一/空feedback、multiplier範囲）
- BurstDetector 6テスト（初期状態、ロスなし、burst検出、sustained検出、閾値以下、回復）
- BitrateController +5テスト（burst抑制、sustained減速、UNDERUSE増速、天井clamp、データなし）
- congestion_control 3テスト（デフォルト、無効値、"loss"モード）
- AdaptiveFEC +3テスト（boost、レート制限、bandwidth_delta）

## [2.2.0] - 2026-04-10

### Added
- **適応FEC:** パケットロス率に応じてFEC冗長度を5-40%で自動調整。`AdaptiveFecController`がBandwidthEstimatorと連携
- **TCP再接続強化:** `DisconnectReason` enum（ClientRequested/ConnectionLost/ProtocolError）で切断理由を識別。ConnectionLost時は5秒間再接続待機
- **セッションログ:** JSONL形式のストリーミング統計記録（10秒間隔、60秒フラッシュ、7日ローテーション）
- **Protocol v3:** TRANSPORT_FEEDBACK (0x12) メッセージタイプ、FVPヘッダにslice_index/slice_count/stream_idフィールド追加
- **Protocol v3互換ゲート:** `fvp_flags::encode_compat()`でv2クライアントにはkeyframeビットのみ送信（後方互換性保証）
- **Adaptive FEC無効化オプション:** `adaptive_fec_enabled = false`で固定冗長度モード（デバッグ用）
- **メモリ監視:** `metrics/memory.rs` — GetProcessMemoryInfo (Win) / /proc/self/status (Linux) でプロセスRSS監視、1時間50MB超過で警告
- **SECURITY.md更新:** TCP再接続5秒PINスキップウィンドウの脅威モデル・緩和策を追記

### Changed
- **chronoクレート導入:** session_log.rsのカスタムISO 8601タイムスタンプをchrono::Utcに置換（カレンダー計算バグ根絶）
- **AdaptiveFecController:** ハードコード初期値20%を廃止、config.fec_redundancyを初期値として使用
- **engine.rs リファクタ:** ストリーミングループからupdate_adaptive_bitrate/check_sleep_mode/update_latency_atomics/log_periodic_statsを関数抽出

### Fixed
- **FEC config検証:** fec_redundancyが[min, max]範囲外の場合にクランプ + 警告ログ
- **FECテストコメント:** boundary_5_percent テストが>=5%ブラケットに入ることを正確に明記

### Tests
- **テスト263件に増加**（180→263、+83件）
- 適応FEC 12テスト（低/中/高ロス、ステップ制限、NaN、境界値、初期値クランプ）
- DisconnectReason 5テスト（ClientRequested、ProtocolError、enum一意性、TransportFeedback正常/異常）
- セッションログ 7テスト（ディレクトリ作成、書込、Drop、空フラッシュ、タイムスタンプ、ローテーション）
- メモリ監視 4テスト（ベースライン、ポーリング間隔、RSS取得、閾値ロジック）
- Protocol v3互換ゲート 3テスト（v1/v2/v3）
- FEC config検証 2テスト（範囲外クランプ、NaN）
- TransportFeedback 5テスト（ラウンドトリップ、空、oversized、truncated、too_short）
- FVP flags 4テスト（simple、full、max、overlap）

## [2.1.0] - 2026-04-07

### Added
- **FT表情プロファイル:** アバターごとの51ブレンドシェイプ感度調整（JSON保存/読込/削除）。OscBridgeがEMAスムージング後にweight適用
- **FT自動キャリブレーション:** 2ステップガイド式（リラックス→誇張）でmin/max収集、自動weight計算。CALIBRATE_START (0x60) / CALIBRATE_STATUS (0x61) プロトコル
- **フォベアテッドプリセット:** subtle (+3/+8)、balanced (+5/+15)、aggressive (+8/+25)、custom。`foveated.preset` config
- **GoogleTest基盤:** driver/CMakeLists.txtにGoogleTest v1.15.2追加。QPマップ計算テスト7件
- **QPマップ純粋関数化:** `computeQpDeltaMap()` を `qp_map.h` に抽出（テスト可能、NVENC非依存）

### Changed
- **FoveatedConfig:** preset enum追加、`effective_qp_offsets()` でプリセットから実効値を解決
- **OscBridge:** プロファイルweight適用対応、`set_profile()` メソッド追加

### Tests
- **テスト180件に増加**（168→180、+12件）
- FT表情プロファイル6テスト（デフォルト、weight、正規化、serialize、roundtrip）
- FTキャリブレーション6テスト（ステップ遷移、フレーム収集、full flow、定数値、index）
- C++ QPマップテスト7件（CTUグリッド、中心/角gaze、プリセット、サイズ検証）

## [2.0.0] - 2026-04-07

### Strategy
- **差別化先行戦略:** VIVE Hubが既に提供するDP/ハンドトラッキング/パススルーより、独自価値（レイテンシー最適化、FT強化、オープンソース）を優先
- **フェーズ再編成:** Phase 1=レイテンシー基盤、Phase 2=Foveated+FT Suite、Phase 3=ハードウェアパリティ

### Added
- **96fpsサポート:** RTPタイムスタンプをconfigフレームレートから動的計算。30-120fps対応
- **プロトコルバージョニング:** HELLO/HELLO_ACKにu16 protocol_version追加。未知メッセージは警告+スキップ（後方互換性維持）
- **UDPトランスポート最適化:** SO_RCVBUF/SO_SNDBUF 2MB + DSCP EF marking（非致命的フォールバック）
- **フルRGBカラーレンジ:** `video.full_range` config + FvpConfig FFI。NVENC VUIパラメータは実機検証待ち
- **レイテンシーウォーターフォール:** HMD内でencode/network/decode/renderの内訳を色分けバーで表示
- **HEARTBEAT_ACK:** PC側エンコード/トータルレイテンシーをHMDに送信

### Fixed
- **RTPタイムスタンプバグ:** `engine.rs:682`の`/90`ハードコードを修正。96fps/120fpsで正しいタイムスタンプを生成
- **フレームレート依存定数:** ビットレート調整間隔、ログ間隔、LatencyTrackerウィンドウをconfigから動的計算

### Changed
- **Config validate():** `Vec<String>` → `Vec<ConfigError>` に変更。構造化されたフィールド名付きエラー（graceful migration維持）

### Tests
- **テスト168件に増加**（156→168、+12件）
- RTPタイムスタンプ回帰テスト3件（90/96/120fps）
- ビットレート調整間隔スケーリングテスト1件
- プロトコルバージョニングテスト3件（encode/decode、空ペイロード、部分ペイロード）
- Config validation構造化エラーテスト更新

## [1.3.0] - 2026-04-07

### Added
- **コンフィグバリデーション:** bitrate/ports/framerate/smoothing/timeout の範囲チェック。不正値はデフォルトにフォールバック+ログ警告
- **コンパニオンアプリ設定UI:** 睡眠モード（enable/timeout）とFace Tracking（enable/smoothing）をGUIから設定可能に
- **サブシステムステータス表示:** Home画面にFT Active/Idle、Awake/Sleep、Audio OK/Off、Packet Loss%をリアルタイム表示
- **エラー通知改善:** ハプティクスドロップカウンター（AtomicU64）、オーディオ状態フラグ（AtomicBool）
- **HMDダッシュボードオーバーレイ:** VR内からビットレート調整・codec確認が可能な設定パネル
- **CONFIG_UPDATEプロトコル:** HMD→PC設定変更メッセージ（0x55）+ ACK（0x56）、値バリデーション付き
- **Atomic status.json:** temp+rename による部分読み取り防止

### Fixed
- **バージョン文字列:** "v1.0.0" 固定 → `CARGO_PKG_VERSION` から自動取得

### Tests
- **テスト156件に増加**（144→156、+12件）
- ハプティクスパイプライン5テスト（シリアライズ、チャネル満杯、roundtrip）
- コンフィグバリデーション7テスト（範囲外、NaN、ポート競合、エッジ値）

## [1.2.0] - 2026-04-07

### Fixed
- **Face Tracking接続修正:** FACE_DATA (0x35)のTCPハンドラが未実装でFTが完全に動作していなかった問題を修正。OscBridgeへのデータパスを接続
- **バッテリーレベル:** コントローラー状態のバッテリー値が100%固定だった問題を修正。Android sysfsから実値を読み取り

### Added
- **Face Tracking EMAスムージング:** blendshape値に指数移動平均フィルタを適用しジッター低減。係数はconfig設定可能（デフォルト0.6）
- **ハプティクスフィードバック:** SteamVR→PCドライバ→TCP→HMDの完全な振動パイプライン。`HAPTIC_EVENT (0x38)`プロトコルメッセージ、OpenXR `xrApplyHapticFeedback`
- **タッチセンサー:** trigger_touch、grip_touch、thumbstick_touch、thumbstick_clickをポーリング・SteamVRに送信
- **HTC VIVE Focus 3コントローラープロファイル:** フル入力バインディング（トリガー/グリップ/スティック/A/B/X/Y/タッチ）+ simple_controllerフォールバック
- **サムスティックデッドゾーン:** 0.1マグニチュード以下をゼロにクランプしドリフト防止
- **VR睡眠モード:** ヘッドポーズの動き検知で非活動検出。タイムアウト後にビットレート低下（80→8Mbps）+ 画面暗転。動き検知で自動復帰
- **[face_tracking]設定セクション:** enabled、smoothing、osc_port
- **[sleep_mode]設定セクション:** enabled、timeout_seconds、motion_threshold、sleep_bitrate_mbps

### Tests
- **テスト144件に増加**（134→144、+10件）
- Face Dataパーステスト、EMAスムージングテスト、SleepDetectorテスト5件

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

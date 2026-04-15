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

## v2.0 スコープ（Phase 1完了）

### ~~96fpsサポート~~ (完了)
- RTPタイムスタンプ`/90`ハードコード修正、フレームレート依存定数を動的化
- ドライバー側は既にconfigからrefresh_rateを広告済み

### ~~プロトコルバージョニング~~ (完了)
- HELLO/HELLO_ACKにu16 protocol_version追加、後方互換性維持
- 未知メッセージタイプはログ警告+スキップ

### ~~UDPトランスポート最適化~~ (完了)
- SO_RCVBUF/SO_SNDBUF 2MB + DSCP EF marking
- 非致命的フォールバック（setsockopt失敗時は警告のみ）

### ~~フルRGBカラーレンジ~~ (完了)
- `video.full_range` config + FvpConfig FFI追加
- NVENC VUIパラメータ（videoFullRangeFlag）は実機検証待ち

### ~~レイテンシーウォーターフォール~~ (完了)
- HMD overlay: encode/network/decode/render色分けバー表示
- HEARTBEAT_ACKでPC側レイテンシーデータをHMDに送信

### ~~Config validation構造化~~ (完了)
- validate() → Vec<ConfigError>、フィールド名付き構造化エラー
- graceful migration維持（クランプ+警告）

## v2.1 スコープ（Phase 2完了）

### ~~FT表情プロファイル~~ (完了)
- profiles.rs: FtProfile構造体、JSON save/load/list/delete、51 weightベクトル
- OscBridgeにweight適用統合、set_profile()メソッド
- config: face_tracking.active_profile フィールド追加

### ~~FT自動キャリブレーション~~ (完了)
- calibration.rs: CalibrationState、2ステップ（Relax→ExaggerateAll）、min/max収集
- weight計算: 1.0 / (max - min)、range < 0.01は1.0フォールバック
- プロトコル: CALIBRATE_START (0x60), CALIBRATE_STATUS (0x61)

### ~~フォベアテッドプリセット~~ (完了)
- FoveatedPreset enum: subtle/balanced/aggressive/custom
- effective_qp_offsets()でプリセット→QP値解決
- qp_map.h: computeQpDeltaMap()純粋関数化

### ~~GoogleTest導入~~ (完了)
- driver/CMakeLists.txt: GoogleTest v1.15.2 via FetchContent
- driver/tests/test_qp_map.cpp: 7テスト（CTUグリッド、gaze、プリセット）

### Foveated Transport (NVENC ROI) — 実機待ち
- **What:** NVENC ROI encodeで視線領域ごとの解像度制御。帯域40%削減目標
- **Why:** プリセット（aggressive +8/+25）で~30%まで改善済み。ROIでさらに40%目標
- **Context:** NVENC SDK 12.x以降が必要。非対応時は現行プリセットにフォールバック
- **Depends on:** 実機でNVENC ROI対応確認

### FTミラーモード — 実機待ち
- **What:** HMD内で自分の表情をリアルタイムプレビュー
- **Why:** キャリブレーション結果の視覚的確認
- **Context:** HMD側カメラフィードが必要。OpenXR passthrough拡張依存
- **Depends on:** 実機入手

## v2.2 スコープ（実機入手後）

### ~~Protocol v3 flags bit layout + 後方互換ゲート~~ (完了)
- fvp_flags::encode_compat()でnegotiated versionに基づきv2互換/v3フルflags切替を実装
- v1/v2クライアントにはkeyframeビットのみ送信。テスト3件追加。

### ~~メモリ監視: staticlibアロケータ問題~~ (完了)
- metrics/memory.rs: GetProcessMemoryInfo (Win) / /proc/self/status (Linux) でプロセスRSS取得
- MemoryMonitor構造体: 60秒ポーリング、1時間で50MB以上増加→警告ログ
- config: [memory_monitor] enabled/poll_interval_seconds/growth_threshold_mb

### ~~TCP再接続PINスキップ: SECURITY.md脅威モデル更新~~ (完了)
- SECURITY.mdのKnown Limitationsテーブルに5秒PINスキップウィンドウの脅威分析を追記
- TLS session resumption + TOFUピニングによる緩和を明記

### ~~GCC Lite（遅延ベース帯域推定の簡易版）~~ (完了)
- BandwidthEstimator.process_feedback() + delay_gradient() 実装済み
- BitrateController.adjust()に遅延判定(OVERUSE/UNDERUSE)を統合済み
- engine.rsでTRANSPORT_FEEDBACKを受信しBandwidthEstimatorに接続済み

### ~~GccEstimator独立モジュール~~ (完了)
- adaptive/gcc_estimator.rs: DelayTrend状態判定、bitrate_multiplier、プロービング準備
- BandwidthEstimatorから遅延計算を分離、テスト7件

### ~~BurstDetector~~ (完了)
- adaptive/burst_detector.rs: LossPattern(None/Burst/Sustained)、500ms閾値
- BitrateControllerとAdaptiveFecControllerに統合、テスト8件

### ~~BitrateController max reductionバグ修正~~ (完了)
- 累積バグ解消、max reduction方式に変更、テスト3件追加

### ~~congestion_control設定トグル~~ (完了)
- config.toml: congestion_control = "gcc" | "loss"、デフォルト="gcc"
- "loss"モードではGCC/Burst無効化、ロスベースのみ

### ~~sent_packet_log~~ (完了)
- engine.rs: HashMap<u16, u64>で送信タイムスタンプ記録、5000エントリ上限

### ~~AdaptiveFecController拡張~~ (完了)
- boost機能(activate/deactivate)、1秒レート制限、bandwidth_delta_from_default()
- transport/fec.rsに維持（Outside Voice指摘の循環依存を回避）、テスト3件追加

### ~~スライスベースFEC~~ (完了)
- Server側: SliceSplitter + encode_frame_sliced (pipeline.rs)、4独立FecEncoder、fvp_flags統合
- Client側: SlicedFecFrameDecoder (fec_decoder.cpp)、4独立RSコンテキスト、u32 length prefix、100ms timeout
- Config: slice_fec_enabled / slice_count、MIN_SLICE_SIZE = 16KB
- IDR_REQUESTレート制限 (max 2/sec) 追加
- テスト17件追加（Rust側）、313 tests total

### ~~TCP再接続holdのステートフル化~~ (完了)
- hold中にTCPリスナーを再作成しHMDが再接続可能に
- accept_failures(5回で停止)とreconnect_attempts(10回で警告のみ)を分離
- Wi-Fi断でエンジンが永久停止するリスクを解消
- 残: TLS session resumption（session ticket）による再接続時PINスキップは実機テスト後

### ~~Adaptive FEC無効化オプション~~ (完了)
- config.toml: adaptive_fec_enabled（デフォルトtrue）を追加
- engine.rs: false時はAdaptiveFecControllerを生成しない（None）、固定fec_redundancyを使用

### Client側FecDecoder RSキャッシュ化 — 実機プロファイル後
- **What:** Android C++ `fec_decoder.cpp`のReedSolomonインスタンスをスライスFEC導入に合わせてキャッシュ化
- **Why:** スライスFECでRS初期化が4倍に増加。現在は毎デコード呼び出しでnew()。RSキャッシュで受信側遅延を削減
- **Context:** サーバー側FecEncoderは既にRSキャッシュ済み（`fec.rs`）。同パターンをC++側に適用。autoplan eng reviewで指摘
- **Depends on:** スライスベースFEC実装完了 + 実機でのプロファイル確認

### Thermal Governor — 実機待ち
- **What:** NVML API経由でGPU温度を監視し、過熱時に品質を段階的に制限
- **Why:** 4時間連続稼働でGPU過熱→フレームドロップ→ユーザー体験悪化を防止
- **Context:** nvml-wrapper crateをoptional依存として追加。spawn_blockingでポーリング。NVML非対応環境では無効化。温度閾値: 75°C警告, 85°C制限, 90°C緊急
- **Depends on:** 実機でのGPU温度プロファイル確認

## v3.0 スコープ（Phase 3+ — 未着手）

### コミュニティプラグインAPI
- **What:** TCPコントロールチャンネルにカスタムデータチャンネル追加。コミュニティが独自モジュール作成可能に
- **Why:** オープンソースの真の強みはエコシステム。ボディトラッキングリレー、カスタムオーバーレイ等が可能に
- **Context:** メッセージタイプ空間(0x00-0xFF)に十分な余裕。カスタムメッセージ登録+コールバックフック+ドキュメントが必要
- **Priority:** P3
- **Depends on:** v2.x完了 + コミュニティ形成

### NVENC VUIパラメータ実機検証
- **What:** NV_ENC_CONFIG_HEVCのVUIパラメータ（videoFullRangeFlag, colorMatrix）のオフセットを実機で検証
- **Why:** インライン構造体のreservedフィールド内オフセットが不正確だとクラッシュの可能性
- **Context:** `video.full_range = true` は設定済みだがNVENC側の設定が未接続
- **Priority:** P2
- **Depends on:** 実機入手

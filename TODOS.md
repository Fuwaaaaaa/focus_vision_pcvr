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

### ~~Foveated Transport (NVENC ROI)~~ — v3.0 で除外、post-v3.0 へ
- **判断:** v3.0 RC 範囲外として明示的に除外。ROI 経路は実機で `nvEncGetEncodeCaps(NV_ENC_CAPS_SUPPORT_EMPHASIS_LEVEL_MAP)` を問い合わせ、対応 GPU + ドライバの組み合わせでしか有効化できない。シミュレータでは検証不能で、書いても確かめようがない
- **現状:** QP delta map (aggressive +8/+25 プリセット) で ~30% 帯域削減を達成済み。実用上ROIなしでも十分機能している
- **削除済み:** `queryRoiCapability()` (always false のスタブ) と `m_roiSupported` メンバ、`isRoiSupported()` getter、`foveatedModeStr()` の roi 分岐をコードから除去 (commit C2)
- **再開条件:** Focus Vision 実機 + NVENC SDK 12.x 対応 GPU が同時に揃った時点で新規 issue で再着手

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

### ~~TCP再接続PINスキップ: SECURITY.md脅威モデル更新~~ (完了 → 2026-09 に訂正)
- SECURITY.mdのKnown Limitationsテーブルに5秒PINスキップウィンドウの脅威分析を追記
- TLS session resumption + TOFUピニングによる緩和を明記
- **訂正 (2026-09-25):** PIN スキップも TLS session resumption も実装されていなかった。実際の動き (hold の 5 秒間だけセッションの PIN を再び受け付ける) に SECURITY.md を合わせた。

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
- accept_failures(5回で停止)とreconnect_attempts(10回で警告のみ)を分離 (2026-09-29: accept_failures でも止めず、エラーを出して再試行を続けるように変更)
- Wi-Fi断でエンジンが永久停止するリスクを解消
- ~~hold 中に再接続を受け付けても、その接続を捨てて新しい PIN で listen し直していた~~ (2026-09-25 修正): hold はセッションの server を再利用してセッションの PIN を 5 秒だけ再び受け付け、再接続した接続でストリーミングを続ける (`headless_e2e_reconnect_within_hold_keeps_pin_and_streams`)
- 残: PIN を使わない再接続 (TLS 内で渡す再接続トークン、または TLS session resumption) はプロトコル変更とクライアント実装が要る。hold の 5 秒が進行中のハンドシェイクを途中で切ってしまう問題 (bind と accept を分ける) も未対応

### ~~Adaptive FEC無効化オプション~~ (完了)
- config.toml: adaptive_fec_enabled（デフォルトtrue）を追加
- engine.rs: false時はAdaptiveFecControllerを生成しない（None）、固定fec_redundancyを使用

### ~~Session Recording MVP~~ (完了)
- recording/mod.rs: Recorder (raw Annex B .h265/.h264)
- recording/audio.rs: AudioRecorder (16-bit PCM WAV)
- [recording] enabled/output_dir config
- engine::write_recording_nal() tap + audio PCM tap (Opus encode 前)
- best-effort (I/O エラー時 poisoned フラグ、streaming は継続)
- Companion UI toggle (runtime CONFIG_UPDATE) は次フェーズ

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

## v2.2.2 スコープ（作業中 / 近日予定）

### ~~リファクタバンドル（Quick wins + Config + Perf/保守性）~~ (12 PR 完了)
- PR #14-20, #22-25, #28: osc_bridge EMA helper, FtProfile::validate, status.json serde_json, session_log write_all, BurstDetector cfg(test), config validate_range, config Default consistency, RTP header helper, sent_packet_log periodic, TCP handshake step, Recording MVP
- PR #21: CI clippy too_many_arguments fix（pre-existing warning を #[allow] で解消）
- PR #29, #30: pipeline.rs / depacketizer を write_rtp_header + write_fvp_header + read_fvp_header に統一

### 次フェーズ候補（未着手、実機待ちではない）
- ~~engine.rs run_streaming() 分割（373 LoC → session_loop / frame_loop / reconnection）~~ (2026-09-25 完了: `StreamingLoop::{run, accept, run_session, hold}`)
- ~~engine.rs adaptive bitrate lock 整理（AdaptiveState struct 化）~~ (2026-09-25 完了: frame loop が持つ `AdaptiveState`、TCP 制御からは `ControlEvent` の mpsc)
- FFI 型重複解消（TrackingData / ControllerState を common に統合）
- CompanionApp 責務分離（tab ごとに別モジュール）
- Dynamic Resolution Scaling (DRS) - adaptive bitrate の延長
- Hand Tracking (OpenXR XR_EXT_hand_tracking)
- Community Plugin API (v3.0)

## AI Super Resolution (resolution_scale) フォローアップ
（2026-06-02 /plan-eng-review で起票。設計doc: AI Super Resolution — HMD側アップスケール）

### エンコーダの縮小を client caps でゲートする（/review 検出 → 安全ガード適用済み）
- **検出:** `encode_stream_config` は caps でゲートするが、`build_fvp_config`→`FvpConfig.encoded_*` は global scale 算出（caps 非依存）だった → scale<1.0 + 非caps クライアントで STREAM_CONFIG=native と driver encode=scaled の不一致。
- **適用済み(安全ガード):** `build_fvp_config` で **encoded_* を当面 native に固定**（エンコーダは縮小しない）。これで scale<1.0 でも不一致・壊れ encode はゼロ。STREAM_CONFIG ネゴ/帯域測定は別経路で不変。`test_build_fvp_config_encoder_stays_native_until_gated` で検証。
- **残り(縮小を実際に有効化する条件):** ①per-session で接続クライアントの caps を見てエンコーダ解像度を確定（再init）②T7 縮小blit。両方が揃った時点でガードを外す。いずれも実機/制御接続配線(P0)依存。
- **Priority:** P1（scale<1.0 を有効化する前に必須）

### ~~bitrate 公式の不整合（pre-existing・/review 検出）~~ (2026-09-25 修正)
- **What:** driver の `bitrate = width*height*2` は 1832×1920 で **~7Mbps** だが、config `bitrate_mbps=80`（STREAM_CONFIG で client へも 80 を送信）。約11×の乖離。fallback 分岐は `80'000'000`（=80Mbps）で経路間も不整合。
- **解決:** `[video] bitrate_mbps` を唯一の値にした。`FvpConfig.bitrate_bps` を追加し、driver はそれを NVENC に渡す (0 なら 80 Mbps)。`bitrate_pixel_factor` は非推奨で無視 (古い設定ファイルは読めて、警告が出る)。
- **残り:** 実際の NVENC のレートは実機で確認する。ただし末尾の P0「ドライバが NVENC を初期化しない」が直るまで、エンコーダ自体が動かない。

### ⚠ BLOCKER (P0): Android クライアントにストリーミングセッション統合が無い
- **調査結果(2026-06-02 read-only 全走査):** 単なる connect/handshake の配線漏れではなく、**「PC に接続してストリーミングを開始する」オーケストレーション自体が未実装**。
  - `initialize()` が init するのは renderer/timewarp/overlay/heartbeat/facialTracker/videoDecoder のみ。
  - `m_networkReceiver.init()` 未呼出 → `mainLoop()` の `if(!m_networkReceiver.isInitialized()) return;`(openxr_app.cpp:265) で UDP 受信が毎回早期 return。
  - `m_tcpClient.connect()/handshake()`・`m_trackingSender.init()` いずれも未呼出。
  - **サーバアドレスの供給元が皆無**(UI/config/ハードコードIP/JNIブリッジ なし)。`MainActivity.kt` は NativeActivity + loadLibrary のみ。
  - git 履歴上、connect/handshake を app に配線したコミットは存在しない。
- **意味:** 実 Android クライアントは end-to-end で未動作の足場。動作実証は Rust simulator(mock_client+headless) で行われている(実機なし前提と整合)。
- **配線に必要(規模):** ①サーバアドレス取得(HMD UI/config/discovery=UX設計) ②セッション・オーケストレータ(connect→handshake(PIN)→receiver/sender init→run + 再接続) ③PIN入力UX。**OpenXR+MediaCodec+実接続を要し実機なしで検証不能** → Phase 1 と同じハード制約でブロック。
- **進捗(2026-06-02):** 純粋部はローカル完結検証で前進:
  - ②**オーケストレーション・ポリシー**: `client_session.h` の `ClientSession` 状態機械(Disconnected→Connecting→Pairing→Configuring→Streaming→Reconnecting、PIN拒否は再接続しない、指数backoff base1s×2 cap16s = engine reconnect.rs と一致)。client gtest 9件。
  - ①**サーバアドレス検証**: `client_session.h` の `parse_server_endpoint("ip"/"ip:port")`(IPv4 4オクテット + port 1-65535 検証、default 9944 = engine と一致)。client gtest 5件。アドレス文字列の**供給元**(config ファイル読込 or UI)は別途。
  - ~~**残り(実機検証が要る部分)**: 状態機械を**実I/O(connect/handshake/receiver init)に配線**~~ (2026-09-28): `StreamSession`(接続・ペアリング・ハートビート・サーバから届くメッセージ・再接続を専用スレッドで回す) と `VideoReceiver`(UDP 受信スレッド + `FrameAssembler`) を追加した。C++ クライアントを本物のエンジン(ヘッドレス版)につなぐ E2E テスト `client/tests/test_session_e2e.cpp` で、TLS 1.3・PIN・映像の受信・誤った PIN・hold 内の再接続・証明書の不一致を確かめた(CI の `client-e2e` ジョブ)。
  - ~~`openxr_app` への組み込み~~ (2026-09-28): 起動時のアドレスと PIN(`am start --es fvp_server … --es fvp_pin …` → MainActivity がファイルに書き、ネイティブ側が読んで消す)、MediaCodec へのフレーム投入(codec・サイズの変更で作り直し)、tracking sender、音声の受信と再生、スリープの減光、終了時の DISCONNECT。APK のビルドまで確認。
  - ~~コンパニオンが adb で起動するときにアドレスと PIN を渡すこと~~ (2026-09-28): Home タブの「Send PIN to headset」と、Deploy のインストール後の起動が、HMD の Wi-Fi のアドレスを調べ、そこへ届く PC のアドレス・エンジンのポート(status.json の `tcp_port`/`udp_port`)・PIN を渡してアプリを起動する(`headset_link.rs`)。
  - **残り**: VR 内の PIN 入力 UI(USB なしで再ペアリングするときに要る)、コントローラ入力の初期化(ハプティクスはそれまで効かない)。デコードと描画、adb からの起動は実機でしか確かめられない。
- **resolution_scale への影響:** PC側(Rust+driver)は機能的。クライアント T8-T10 は正しいが、このセッション統合が無いため inert。
- **Priority:** P0 (クライアント実機動作の前提・Phase 1 より上流)
- **Depends on:** 実機(検証に必須) + UX設計

### Phase 0/1 分離（実装シーケンシング・/plan-eng-review で確定）
- **What:** 本機能を2段に分離。**Phase 0**=半解像度エンコード + caps gate + HMD bilinear直描き(GlslUpscalerなし)。**Phase 1**=GlslUpscaler(bicubic+inline unsharp)を追加。
- **Why:** Phase 0 の帯域削減効果は simulator E2E で NAL サイズを測れば**ハード無しで検証可能**。bicubic/unsharp の bilinear 超えだけが検証不能 → 検証可能部を先に出し、不能部(Phase 1)は実機で bilinear 超えを証明してから着手(incremental/reversible)。
- **Context:** 確定済み10決定はそのまま生きる(GlslUpscaler 関連=Decision 4,6,7,9 のみ Phase 1 へ)。Issue0(scale固定)/Issue2(encoded_w/h)/Issue4(caps gate)/P-1(bitrate_pixel_factor) は Phase 0 に含む。
- **⚠ 決定1↔7 精緻化(外部声指摘):** 実行中(サーマル/メモリ逼迫)に upscaler が死んだ場合、サーバは半解像度送出中のため "native" には戻れない。正しい runtime fallback は **bilinear引き伸ばし + 警告バナー**(画像は出るが鮮鋭化なし)。init失敗だけでなく runtime失敗も同経路で扱うこと。
- **状態:**
  - **Phase 0 = P1・今すぐ着手可（依存なし、ハード無しで検証可能）。**
  - **Phase 1 (GlslUpscaler) = 🔒 HELD — 実機(VIVE Focus Vision)入手まで保留（2026-06-02 ユーザー確定）。** 実機で bicubic+unsharp の bilinear 超え + ≤5ms + 電力 を確認できるまで着手しない。GlslUpscaler 関連の確定済み決定(Decision 4,6,7,9)はこの保留に含まれる。
- **Priority:** Phase 0=P1 / Phase 1=P3(blocked: hardware)
- **Depends on:** Phase 0=なし / Phase 1=実機入手 + Phase 0完了 + 実機検証ガントレットで bilinear 超え実証

### Phase 2: AI超解像 (LiteRT/TFLite delegate)
- **What:** Phase 1 の GlslUpscaler を AIモデル(ESPCN/RealSR等)実装に差し替え、真のAI超解像で品質向上。
- **Why:** 本機能の差別化の核心(ALVR/VD未実装)。モデル選定・量子化・電力が実機検証必須。
- **Context:** CQ-1 決定により Phase 1 は具象 GlslUpscaler 1つのみ。Phase 2 でAI実装が実在したら両者から抽象 Upscaler interface を抽出。詳細は別 /office-hours。
- **⚠ API選定(外部声指摘):** NNAPI は Android 15(2024) で deprecated 方向。**LiteRT(旧TFLite) GPU/NPU delegate** を前提にすること。設計doc記載の「NNAPI」は読み替え。
- **Priority:** P3
- **Depends on:** 実機(VIVE Focus Vision)入手 + Phase 1完了

### 実機検証ガントレット (Phase 1 の Success Criteria 確定)
- **What:** 実機入手時に走らせる検証一式 — bench_upscaler_glsl_latency(adb計測, ≤5ms)、bilinear比較の知覚品質、電力(3hセッション)、VR酔い主観テスト、unsharp amount と bitrate_pixel_factor のチューニング。
- **Why:** 本機能の"価値そのもの"が全てこのガントレットでしか検証できない。Success Criteria 達成可否はここで初めて判明。
- **Context:** Phase 1 実装時に config ノブ(bitrate_pixel_factor)と unsharp amount uniform を出しておくので、再ビルドなしでチューニング可能。（2026-09-25: `bitrate_pixel_factor` は廃止。エンコーダの bitrate は `bitrate_mbps`。縮小時の bitrate が要るなら `build_fvp_config` で面積比から出す）
- **Priority:** P2
- **Depends on:** 実機入手 + Phase 1完了

### ~~クライアント HELLO version 名乗り修正~~ (T8 完了)
- **What:** C++クライアントが HELLO で {1,0}=version 1 を名乗っていた。サーバの PROTOCOL_VERSION は 3。
- **解決:** T8 で HELLO を v3 名乗り + caps バイトに修正(tcp_client.cpp)。調査の結果クライアントは既に v3 ワイヤ形式(fvp_flags slice/stream — fec_decoder.h:70-73)を実装済みのため v3 名乗りは安全。

### ~~client C++ テスト基盤 (host-buildable gtest)~~ (完了)
- **What:** `client/tests/`（host CMake プロジェクト）+ `client/app/src/main/cpp/client_protocol.h`（Android非依存の純粋ロジック）。driver と同じ gtest パターン。
- **解決:** `client_protocol.h` に HELLO ペイロード構築 + 定数を抽出し `tcp_client.cpp` がそれを使用。`client/tests/test_client_protocol.cpp` で単体テスト(3件)。CI に `client-tests`(ubuntu) ジョブ追加、CLAUDE.md に手順記載。`ctest --test-dir client/tests/build` で実行。
- **今後:** T9(STREAM_CONFIG parse)・T10(decoder sizing) の純粋部もこの基盤で TDD する。FVP flag 解析(fec_decoder.h)も将来ここへ取り込み可。

## ⚠ BLOCKER (P0): ドライバが NVENC を初期化しない — 実機では映像が 1 フレームも出ない
（2026-09-25 のコード調査で判明。GPU・SteamVR が無い開発機では検証できないため、直さずに記録）
（2026-09-29: 下の各点と表示コンポーネント・フレームのコピー・bitrate の反映を配線した。D3D11 の部分は WARP の gtest で確認済み。SteamVR と NVIDIA GPU での確認はまだ）
- ~~**What:** `CDirectModeComponent::initEncoder`（`driver/src/direct_mode.cpp`）を呼ぶ場所がどこにもない（`git log -S` でも追加以来ずっと未使用）。`m_encoderReady` は常に false で、`Present()` は毎回早期 return する。~~ (2026-09-29): `CHmdDevice::Activate` が `CDirectModeComponent::init()` を呼び、デバイス・`EyeBlit`・`NvencEncoder` を作る。
- **同じ経路のほかの問題:**
  - ~~driver は D3D11 デバイスを作っていない。`CreateSwapTextureSet` は `m_encoder.getDevice()`（初期化前は null）を使うので、スワップテクスチャも作れない。~~ (2026-09-29): NVIDIA の GPU(なければ最もメモリの多いハードウェア GPU)にデバイスを作り、`Prop_GraphicsAdapterLuid_Uint64` で compositor に同じアダプタを使わせる(`gpu_adapter.*`)。
  - ~~`rSharedTextureHandles` に入れているのは `m_nextHandle++` の連番で、DXGI の本物の共有ハンドル（`IDXGIResource::GetSharedHandle`）ではない。SteamVR の compositor はこれを開けない。~~ (2026-09-29): `SwapTextureSets` が `GetSharedHandle` の値を返す。別デバイスから開けることを WARP で確認。
  - ~~`nvenc_encoder.h` の手書きの NVENC 関数テーブルの並びが、公式の `nvEncodeAPI.h`（`NV_ENCODE_API_FUNCTION_LIST`）と合っていない疑いがある~~ (2026-09-28 修正): 疑いどおりだった。関数テーブルに加えて、構造体バージョンの作り方、`NV_ENC_OPEN_ENCODE_SESSION_EX_PARAMS` / `NV_ENC_INITIALIZE_PARAMS` / `NV_ENC_LOCK_BITSTREAM` / `NV_ENC_RC_PARAMS` / `NV_ENC_PIC_PARAMS` のレイアウト、`NV_ENC_BUFFER_FORMAT_ARGB`（0x20 → 正しくは 0x01000000）、`NV_ENC_PIC_FLAG_FORCEIDR`（4 → 正しくは 2）も違っていた。公式ヘッダ（`third_party/nvenc`、SDK 12.2）に置き換え、preset config を `nvEncGetEncodePresetConfigEx` で取得して設定を上書きする形にした（`driver/src/nvenc_config.h`）。HEVC の QP delta map は 32x32 CTB 単位に直した（64 は NVENC が対応していない）。実機で NVENC が動くかはまだ確認していない。
  - ~~`fvp_set_bitrate_callback` を driver が登録しておらず、`NvencEncoder` に reconfigure（`nvEncReconfigureEncoder`）の経路もない。adaptive bitrate の変更は NVENC に届かない。~~ (2026-09-29): callback を登録し、値を atomic に預けて、次の encode の前に `nvEncReconfigureEncoder` で反映する(IDR なし、rate control のリセットなし。`fvp_nvenc::reconfigureParams`)。失敗したら元の bitrate のまま続ける。
- **直すのに必要なこと:** ~~D3D11 デバイスの作成（SteamVR が使うアダプタ）、本物の共有テクスチャ、`initEncoder` の呼び出し、~~ 関数テーブルの検証（実機）~~、bitrate callback と reconfigure（Present スレッドで適用する atomic な保留値）~~。
- **Priority:** P0（実機での映像の前提）
- **Depends on:** NVIDIA GPU + SteamVR のある環境（ビルドと gtest 以外は検証できない）

## コード調査で見つけた問題（2026-09-25、未対応）
- ~~**C++ クライアントの受信が遅い:** `openxr_app.cpp` は 1 回の render loop で最大 64 パケットしか読まず、ループは `xrWaitFrame` で止まる。約 5.8k パケット/秒が上限で、80 Mbps には足りない。~~ (2026-09-28): `VideoReceiver` が専用スレッドで受信し、FEC の復元もそこで行う。`openxr_app` への組み込みは次の作業(上の P0)。
- ~~**MediaCodec に渡る順番が入れ替わる:** 完成したフレームは、同じ decoder が次のフレームを見たときか flush のときにしか渡されず、flush は bulk → sliced の順。~~ (2026-09-28): `FrameAssembler` は、bulk でも sliced でも、そろった時点でフレームを渡すので、送った順に出る。
- **RS の行列をほぼ毎フレーム作り直している:** `FecEncoder` のキャッシュはデータ shard 数が前回と同じときしか効かず、実際のエンコーダ出力はフレームごとにサイズが変わる。IDR 級のスライス（約 180 shard）では行列の作成が重い。スライスをまたいで共有する `ReedSolomon` の LRU があるとよい。
- **bitrate を変える経路がばらばら:** adaptive controller、sleep（起きると controller の値ではなく `bitrate_mbps` に戻る）、CONFIG_UPDATE 0x01（controller を通らない）、thermal（`notify` しない）がそれぞれ別に動いている。1 つの実効値（min(controller, sleep, thermal, user)）にまとめたい。`GccEstimator::set_current_bitrate` はどこからも呼ばれていない。
- ~~**`config.pairing.max_attempts` / `lockout_seconds` が使われていない:** `PairingState` は定数を直接使う。~~ (2026-09-30 修正): 設定の値でロックする。総当たりが既定の 2 倍より速くならないよう、1〜10 回・300〜3600 秒に制限した。
- ~~**tcp-control タスクが cancel されない / `HAPTIC_TX` が残る:** UDP sender の作成失敗やフレームの供給元が閉じたことでセッションを抜けても、`handle_tcp_control` は HMD が TCP を切るまで動き続ける (その場合の切断理由は ConnectionLost 扱い)。セッション終了後も `HAPTIC_TX` に古い sender が残る。~~ (2026-09-25 修正): `run_session` を抜けると、どの経路でもセッションの cancel が発火し (drop guard)、tcp-control タスクは接続を閉じて終わる (`spawn_control_task`)。`HAPTIC_TX` は `HapticRoute` がセッションの間だけ設定する。フレームの供給元が閉じたときは、ConnectionLost ではなく engine の停止として扱う。
- **UDP のパケットごとの認証がない:** tracking の送信元チェックは IP だけ（SECURITY.md の Known Limitations に記載済み）。

## 2026-09-28 監査で見つかった問題（未対応）
（コードを読んで見つけたもの。実機では確認していない。同じ監査で見つかったもののうち、PIN の回避、依存の脆弱性と CI の監査、Android アプリの起動時クラッシュ、インストーラの DLL 配置とフォント、NVENC の定義は修正済み。CHANGELOG [Unreleased] を参照）

### ドライバ（上の P0 に加えて、映像を出すまでに要るもの）
- ~~**P0: 表示コンポーネントがない。** HMD は DirectMode コンポーネントしか返さず（`hmd_device.cpp` `GetComponent`）、`IVRDisplayComponent` がない。投影・レンダーターゲットの大きさ・目ごとのビューポートを compositor が得られない。~~ (2026-09-29): `CDisplayComponent`。レンダーサイズは `render_width/height`、歪みなし。FOV は固定の既定値(片目 100°、`display_geometry.h`)。
- ~~**P0: フレームコピーが何もしない。** `FrameCopy` は `R8G8B8A8_UNORM`、NVENC の入力は `B8G8R8A8_UNORM` で、形式グループが違う `CopyResource` は D3D11 が無視する。compositor の `nFormat` との一致も要る。~~ (2026-09-29): `EyeBlit` がシェーダで描き写す。レイヤーの bounds(左目・反転)、サイズの違い(SteamVR のスーパーサンプリング)、sRGB / float の元テクスチャを吸収して、NVENC の B8G8R8A8 入力にする。WARP で読み戻して確認。
- ~~**P1: 左目しか送らない**（`direct_mode.cpp` `Present`）。立体視にならない。クライアントも 1 本の映像を両目に出している。~~ (2026-09-29): 左右並び(プロトコル v5)。ドライバは最初のレイヤーの両目を幅 2 倍のエンコーダ入力の左右に描き、STREAM_CONFIG の 26 バイト目で配置を伝える。クライアントは幅 2 倍でデコードし、目ごとに半分を表示する。foveated の QP マップも目ごとに中心を置く。映像の画素数が 2 倍になるので、同じ `bitrate_mbps` では画質が下がる(実機で既定値を見直す)。
- ~~**P1: ヘッドセットの FOV を使っていない。** `CDisplayComponent` の投影は固定の既定値(片目 100°)で、実機の FOV と違う分だけ像の大きさがずれる。~~ (2026-09-29): クライアントが目ごとの FOV と IPD を VIEW_CONFIG (0x22) で送り(接続のたびと変わったとき)、エンジンが検証してドライバのコールバックに渡し、ドライバが `SetDisplayProjectionRaw` / `SetDisplayEyeToHead` / `Prop_UserIpdMeters_Float` で差し替える。あわせて raw projection の top / bottom を OpenVR の規約(`top` = 下端の tan、`bottom` = 上端の tan。ComposeProjection と ALVR の fov_to_tangents で確認)に直した(非対称の FOV で上下が逆になっていた)。既定値は最初の接続までだけ使う。
- ~~**P1: 重ねるレイヤーを合成していない。** `SubmitLayer` の最初のレイヤー(シーン)しか使わないので、SteamVR のダッシュボードやオーバーレイが映らない。~~ (2026-09-29): すべてのレイヤーを下から描く(1 フレーム 16 枚まで)。シーンは不透明(アルファ 0 で出すアプリがある)、その上のレイヤーはアルファで重ねる(ALVR と同じ)。シーンと違う頭の向きで描かれたレイヤーは、目の FOV を使ってピクセルごとにシーンの向きへ回す(`layer_compose.h`、host テストあり。シェーダは WARP で確認)。回転だけで、頭の位置の差は無視する。実機では未確認(オーバーレイのアルファが straight か premultiplied かも実機で確かめる)。
- ~~**P1: `syncTexture` を使っていない。** keyed mutex の取得も vsync イベントもないので、描画途中のフレームを読む可能性がある（実機で確認が要る）。~~ (2026-09-29): `Present` は sync texture の keyed mutex を取ってから読む(10 ms 待って取れなければそのフレームを飛ばす)。vsync は SteamVR に任せ、`PostPresent` でフレームの枠の終わりまで待ってペースを作る(`FramePacer`。ALVR の現行の方式。最初は自前の vsync イベントをスレッドから送っていた)。実機でのタイミングは未確認。
- ~~**P1: NVENC が使えないとき、黙ってダミー NAL（IDR ヘッダ + `0xAB`）を流す。** `init()` は true を返し、失敗は `OutputDebugStringA` にしか出ない（vrserver.txt に残らない）。失敗として扱い、`VRDriverLog` に出すべき。~~ (2026-09-29): `init()` は false を返し、理由を vrserver.txt に出す。映像は流さない。
- ~~**P1: コントローラの入力プロファイルがない。** `{focus_vision_pcvr}/input/controller_profile.json` が存在せず、`Prop_ControllerType_String` も未設定なので、SteamVR 側にボタン割り当てがない。~~ (2026-09-29): ユーザーの決定で Oculus Touch(Quest 2)として見せる(ALVR / Virtual Desktop と同じ)。controller_type `oculus_touch`、SteamVR の Oculus ドライバの `{oculus}/input/touch_profile.json` と描画モデル。ボタンは左が X/Y、右が A/B、メニューと VIVE ボタンは Touch の system。ゲームの Touch 用の既定の割り当てがそのまま効く(`touch_profile.h`)。
- ~~**P1: 終了時の use-after-free。** `server_driver.cpp` `Cleanup` がデバイスを破棄してから `fvp_shutdown()` を呼ぶため、tracking タスクの gaze / IDR コールバックが破棄済みの `m_hmdDevice` を触りうる。`fvp_shutdown` を先に呼ぶ。~~ (2026-09-29): `fvp_shutdown` を先に呼ぶ。
- P2: ~~`DestroySwapTextureSet` がセットの 3 枚のうち 1 枚しか消さない。`initEncoder` が `full_range` と foveation の設定を無視する。head pose が一度来ると以後ずっと「有効」、~~ (2026-09-29 修正。head pose はエンジンがセッション終了時に消す #18) 速度が未設定(クライアントが速度を送っていないので、トラッキングのプロトコルの拡張が要る)。~~`GetPose` が同期なしで読む。CONFIG_UPDATE の codec 変更 (0x02) は受理を返すが何もしない。~~(2026-09-29 修正: 姿勢はミューテックスの中で更新・コピーする #35。codec 変更は「拒否」を返す #38)

### Android クライアント（上の P0 のセッション配線に加えて）
- ~~**P1: サーバから届くメッセージを読まない。**~~ (2026-09-28): `StreamSession` のスレッドが読み、`ServerEvent` としてアプリに渡す。TLS の読み書きはそのスレッドだけが行う。アプリ側の処理(ハプティクス・スリープ表示)は `openxr_app` への組み込みと一緒に行う。
- **P1: HEARTBEAT のロス統計が常に 0。** (2026-09-28 一部修正): `FrameAssembler` が RTP シーケンス番号の抜けをロスとして数え、`StreamSession` が 500 ms ごとに実際の値と実時間から計算した fps を送る(E2E で HEARTBEAT_ACK まで確認済み)。`onFrameDecoded` はデコーダの配線と一緒に呼ぶ(#16 で済み)。TRANSPORT_FEEDBACK (0x12) は未実装で、GCC は入力なしのまま。実装は実機待ちにする: エンジンの GCC は受信時刻の差をそのまま遅延の傾きとして扱っていて、送信時刻との差にしていない。直してから、実際の Wi-Fi の揺らぎでしきい値を調整しないと、ビットレートを不必要に下げるおそれがある。
- ~~**P1: STREAM_CONFIG の codec を無視する。** デコーダは常に `video/hevc` で、handshake より前に作られる。~~ (2026-09-29 確認): `configureDecoder` が STREAM_CONFIG の codec と大きさでデコーダを作り直す。
- ~~**P1: 描画の問題。** 右目が上下逆になる（Renderer は V を反転、Timewarp はしない）。Timewarp の行列を転置なしで渡し、+Z を前方とし、頂点ごとに透視除算している。~~ (2026-09-29): Renderer と Timewarp を 1 つのシェーダにまとめた。デコード済みフレームは 1 ループに 1 回だけ取り出して両目に使う(目ごとに取り出していたので、左右で違うフレームになっていた)。回転の補正はピクセルごとに、-Z 前方で、行列は転置して渡す。計算は `video_view.h` にあり、host の gtest で確かめる。シェーダは glslang で GLSL ES 3.00 として検証した(実機では未確認)。
- ~~**P1: フレームとそれを描いたときの頭の姿勢を結びつける手段がプロトコルにない。** 回転の補正は、フレームを最初に表示したときの姿勢からの差だけを補う。~~ (2026-09-29): プロトコル v6。ドライバが `SubmitLayer` のシーンの頭の姿勢(`mHmdPose`)を四元数にしてフレームと一緒に渡し(`fvp_submit_encoded_frame`)、エンジンは `hello_caps::FRAME_POSE` を名乗ったクライアントにだけ、フレームの先頭に 20 バイトで付ける(STREAM_CONFIG 27 バイト目)。クライアントはデコード前に外し、デコーダの PTS からフレームを引き直して、新しいフレームも含めてその向きから今の目の向きへ回す。host テスト、headless E2E、C++ クライアントと本物のエンジンの E2E で確認。実機では未確認(SteamVR の `mHmdPose` がクライアントの報告した姿勢と同じ座標系であることは ALVR と同じ前提)。
- ~~**P1: 復元できないフレームを黙って捨て、IDR を要求しない**（bulk / sliced とも）。~~ (2026-09-28): `FrameAssembler` は、欠けたフレーム(シャード不足・届かなかった frame index・100 ms のタイムアウト)があると IDR を要求し、キーフレームが来るまで非キーフレームを渡さない。セッション開始時も同じ。
- ~~**P1: OpenXR 拡張を有効にしていない。** instance は 2 つの拡張しか有効にしないので、`vive_focus3_controller`・facial tracking・eye gaze が使えない。eye tracker と controller poller がそれぞれ `xrAttachSessionActionSets` を呼ぶ（セッションで 1 回しか許されない）。~~ (2026-09-29): ランタイムが持つ任意の拡張(Focus 3 コントローラ・eye gaze・facial tracking)を有効にする(`openxr_extensions.h`、host テストあり)。調べると、コントローラも eye tracker も初期化自体が呼ばれていなかった。`initInput` が両方の action set を作って 1 回の `xrAttachSessionActionSets` で付け、`xrSyncActions` は毎フレーム 1 回。gaze は VIEW 空間で求め、目の画像の中の位置に直す(`gazeInImage`、host テストあり)。実機では未確認。
- ~~**P1: 音声を受信・再生する処理が配線されていない**（RTP の除去も `pump()` の呼び出しもない）。~~ (2026-09-28 #16 で済み): RTP ヘッダを外して `AudioPlayer` に渡し、`pump()` を毎ループ呼ぶ。
- P1（要確認）: ~~FEC の復元を描画スレッドで~~ O(n²·1200) で行う。約 200 shard のスライスで 100 ms 以上止まる可能性。(2026-09-29 確認: #15 以降、復元は `VideoReceiver` の受信スレッドで行うので、描画は止めない。残るのは、大きなスライスの復元中に受信が遅れる可能性で、実機で時間を測る)
- P2: MediaCodec の入力バッファがないと黙ってフレームを捨てる。`releaseOutputBuffer` の直後に `updateTexImage` を呼ぶ。SurfaceTexture の変換行列を無視する。sRGB swapchain にガンマ済みの映像を書く（二重ガンマ）。送る head pose が左目の pose。(2026-09-28 修正: 遅れた 1 パケットで進行中のフレームを捨てる件と、映像 UDP の送信元を確認しない件は `FrameAssembler` / `VideoReceiver` で直した)
- ~~P2（TLS）: PIN 認証の前に fingerprint を保存する。connect / handshake にタイムアウトがない。connect 失敗後の `disconnect()` が未初期化の mbedtls 構造体を解放する。fd を二重に close する。`psa_crypto_init()` を呼んでいない。~~ (2026-09-28 修正): 証明書はペアリング成功後に固定する。接続・TLS・各ハンドシェイク段階・書き込みにタイムアウトを付けた(ノンブロッキングのソケットと `mbedtls_net_poll`)。mbedtls の文脈は常に初期化済みで、ソケットは `mbedtls_net_free` だけが閉じる。`psa_crypto_init()` を呼ぶ。1 メッセージを 1 つの TLS レコードで送る。

### エンジン
- ~~**P1: 制御チャネルに生存確認がない。** `HEARTBEAT_MAX_MISSES` は未使用、TCP keepalive もない。Wi-Fi が黙って切れると、エンジンは「配信中」のまま HMD の再接続を拒み続ける。~~ (2026-09-29): HMD から 3 秒(`HEARTBEAT_INTERVAL_MS` × `HEARTBEAT_MAX_MISSES` = 500 ms × 6)何も届かない制御接続は切断として扱う。クライアントの `livenessTimeoutMs` と同じ 3 秒。制御メッセージの読み取りは cancel-safe な `read` + 受信バッファにした(`read_exact` を select! に入れると、ハプティクス送信が勝ったときに読みかけのメッセージが壊れる)。`headless_e2e_silent_hmd_is_dropped`
- ~~**P1: セッション終了後も tracking / コントローラの状態が残る。** 切断時にスティックを倒していると、SteamVR では倒したままになる。~~ (2026-09-29): セッションが終わると head pose とコントローラの状態を消す(`AuthorizedPeerGuard` の drop。受信側は read lock を持ったまま保存するので、終了後に遅れたパケットが状態を戻すことはない)。250 ms 届かないコントローラは `get_controller` が `None` を返し、ドライバはそのとき入力を一度だけ離した状態にする。`headless_e2e_stale_and_ended_hmd_input_is_released`
- ~~**P1: `accept()` のエラーでエンジンが止まる。** 接続単位のエラーも数えるため、LAN の誰かが接続→RST を 6 回繰り返すと SteamVR の再起動まで止まる。~~ (2026-09-29): 接続単位の `accept()` エラーはログに出して待ち受けを続ける(100 回連続で初めてリスナーの失敗として返す)。リスナーの失敗が続いてもエンジンは止まらず、最大 16 秒間隔で再試行を続ける。
- ~~**P1: 音声キャプチャが 48 kHz ステレオ決め打ち。** 44.1 / 96 kHz の機器では cpal がループバックを開けず無音。5.1 / 7.1 はダウンミックスしない。~~ (2026-09-29): 機器本来の形式で開き、`audio/convert.rs` で 48 kHz ステレオにする(4 / 5.1 / 7.1 はセンターとサラウンドを −3 dB で左右に足し、LFE は捨てる。レート変換は窓付き sinc)。単体テストで確認、44.1 kHz やサラウンドの実機では未確認。音声デバイスの切り替えを追わない件(P2)は 2026-09-30 に修正。
- ~~**P1: フェイストラッキングの OSC 名の並びが `XrEyeExpressionHTC` と違う**（例: index 2 は RIGHT_BLINK なのに `EyeLeftRight` で送る）。舌の並びも違う。キャリブレーションの重みが min を引かない（安静時に 1.0 が出る）。`face_tracking.enabled` を無視する。`+Inf` が NaN ガードを通る。`smoothing = 0` だと 0 に戻らない。~~ (2026-09-29): 名前表を openxr.h の列挙の並びにした(列挙子の名前から組み立てた名前と照合するテストあり。シミュレータとテストも右目のまばたきを 6 番としていたので直した)。プロファイルに安静時の値 `offsets` を足し、キャリブレーションは offset = min、weight = 1 / 幅(古いプロファイルは offset 0 で読める)。非有限の入力は捨てる。閾値を下回ったパラメータは 0 を 1 回送る。`enabled` と `active_profile` を効かせた。パラメータ名そのもの(SRanipal 風)は変えていない。VRChat 上では未確認。
- ~~**P1: 適応ビットレートの上限が 200 Mbps 決め打ちで、`bitrate_mbps` は初期値にすぎない。**~~ (2026-09-29): `bitrate_mbps` を上限にした。回線が悪いと下げ、良くなるとこの値まで戻す(10 Mbps が下限)。ヘッドセットのダッシュボードの変更(CONFIG_UPDATE)もエンコーダに直接ではなくコントローラに渡し、新しい上限にする(直接だと次の調整で元に戻っていた)。熱による上限とスリープからの復帰も、設定ファイルの値ではなくこの上限を基準にする。
- ~~**P1（テストの穴）: モッククライアントは HEARTBEAT ではなく中身ゼロの HEARTBEAT_ACK を送る。** E2E は個数しか見ないため、適応制御の経路が試されていない。~~ (2026-09-29): HMD と同じ形の HEARTBEAT(RTP シーケンス番号の抜けから数えたロス、受信数、fps)を 500 ms ごとに送り、エンジンの HEARTBEAT_ACK を数える。E2E は ACK が返ることも確かめる。
- P2: ~~バースト検出器に EWMA 済みのロスを渡している。ロス報告の半分を捨てる。~~（2026-09-29 修正 #39: 調整までの HEARTBEAT を足し合わせ、バースト検出器にはその区間のロス率を渡す）~~エンコード時間が常に約 0、status.json の fps は設定値。~~（2026-09-30 修正: ドライバがフレームの準備から NVENC の出力までを測って `fvp_submit_encoded_frame` で渡し、PC の遅延はそこから数える。fps は直近 1 秒に送ったフレーム数。実機の NVENC での値はまだ見ていない）~~LowDelay の Opus では in-band FEC が効かない。~~（2026-09-30 修正: CELT 専用のモードには FEC がないので、有効にする設定と「FEC enabled」のログを外した。HMD は RTP のシーケンス番号の抜けを数え（`audio_sequence.h`、host テストあり）、失われた 10 ms を Opus のパケット損失補間で埋める。遅れて届いたパケットは捨てる。実機の音では未確認）~~録音の WAV ヘッダがクラッシュ時に 0、同じ秒の再接続でファイルが上書きされる。~~（2026-09-30 修正: 1 秒ごとにヘッダのサイズを書き、4 GB の上限で止める。既存の名前には `-2`… を付けて新しく作る）~~音声デバイスの切り替えを追わない。~~（2026-09-30 修正: 2 秒ごとに既定の出力デバイスを確かめ、変わったとき・ストリームがエラーを出したときに開き直す。出力デバイスなしで始まったセッションも、つながった時点で開く。実際の切り替えは実機で確かめていない）~~sleep 中のビットレートが次のセッションに残る。再接続の最初の 4 フレームが古く、セッション開始時に IDR を出さない。~~（2026-09-29 修正: セッション開始時に古いフレームを捨て、エンコーダを設定のビットレートに戻し、IDR を要求する。headless E2E あり）~~256 B を超える datagram でログがあふれる。tracking の bind を再試行しない。~~（2026-09-29 修正 #40: 受信バッファ 2 KB、受信エラーのログは間引く。bind は 1〜16 秒の間隔で再試行）(2026-09-29 修正: `set_nodelay` を付け、1 メッセージを 1 回の書き込み = 1 つの TLS レコードで送る)
- P2（設定）: ~~`sleep_bitrate_mbps` 未検証（0 可、4294 超で overflow）。`udp_port` が 65533 以上で u16 の足し算が overflow。`resolution_per_eye` / `ipd` が NaN や 0 を通す。~~（2026-09-29 修正 #33: sleep は 1〜200、udp_port は 65532 まで、目の大きさは 320〜4096 の偶数、IPD は 40〜90 mm、vsync→photons は 0〜0.1 s）効かない設定: ~~`face_tracking.enabled`、`active_profile`~~（2026-09-29 修正）、~~`audio.bitrate_kbps`（128k 固定）、`[memory_monitor]`、SessionLogger（本番で作られない）~~（2026-09-30 修正: Opus のビットレートに反映。メモリ監視は 1 時間ごとに engine.log へ。セッションログは配信中 10 秒ごとに `sessions/*.jsonl` へ書き、7 日で消し、診断 zip に最新 5 件を入れる。あわせて、効かない `audio.frame_size_ms` / `sample_rate` / `channels` は設定ではないと明記し、default.toml から外した）。~~TLS の初期化に失敗すると黙って平文になる。~~（2026-09-29 修正 #32: TLS なしでは待ち受けず、再試行のたびに初期化をやり直す）
- P2: バッファプールを補充しない（毎パケット確保）。fuzz が本番の経路（FEC 付き packetize、`encode_frame_sliced`、`parse_face_data`、HEARTBEAT、CONFIG_UPDATE）を通っていない。

### コンパニオン・インストーラ・CI
- ~~**P1: 日本語が表示されない**（CJK フォントを読み込んでいない。エンジン停止バナーなどが豆腐になる）。~~ (2026-09-29): OS の日本語フォント(游ゴシック Medium → メイリオ → MS ゴシック、Linux / macOS は Noto Sans CJK / ヒラギノ)を各ファミリーの主フォントの直後に入れる(egui 内蔵の代替フォントより前。全角括弧がアイコン用フォントから描かれていた)。egui は混在フォントのベースラインではなく高さの中央をそろえるため、游ゴシックが約 3 px 浮く。`FontTweak::y_offset_factor` をフォントの数値から計算して補正する(`japanese_text_sits_on_the_latin_baseline` が実際のレイアウトで 1 px 以内を確認)。
- ~~**P1: Settings にスクロールがなく、既定の 480×640 では Codec・Export Logs・Reset が見えない。**~~ (2026-09-29): 全タブを縦スクロールにした(タブごとにスクロール位置を持つ)。
- ~~**P1: コンソールウィンドウが開く**（`windows_subsystem` なし。閉じると最後の設定保存が飛ぶ）。~~ (2026-09-29): リリースビルドは GUI サブシステム(デバッグビルドは `cargo run` のログのためコンソールを残す)。adb / PowerShell / wmic は `CREATE_NO_WINDOW` で起動する(`process::command`。これがないとデバイス検出のたびに黒い窓が出る)。
- ~~P2: 最小化したまま(タスクバーから)閉じると、次の起動で 80×103 px の窓になる。~~ (2026-09-29): eframe の `persistence` が最小化中の位置 (-32000, -32000) とサイズ 0×0 を保存し、次回それを復元していた。アプリは eframe の保存領域を使っていないので、機能ごと外した(既存の app.ron は読まれない)。
- ~~**P1: 診断 zip に PC のログがほぼ入らない。** エンジンの `log::` 出力はどこにも記録されず、vrserver.txt も集めない。`wmic` は Windows 11 24H2 以降にない。~~ (2026-09-29): エンジンは `%APPDATA%/FocusVisionPCVR/engine.log` に書く(エンジンは info、依存は warn、`RUST_LOG` で変更可。前回の分と 16 MB を超えた分は `engine.prev.log`)。zip にはそれと status.json、`config/local.toml`、SteamVR の vrserver / vrcompositor のログ(`openvrpaths.vrpath` の `log` から探す)を入れる。system info は CIM で Windows の版と GPU・ドライバの版を出す(この PC にも wmic はなかった)。SteamVR 上での出力は未確認。
- ~~**P1: ファイル選択で日本語のパスが化ける**（PowerShell 5.1 の出力は OEM コードページ）。選択ダイアログが UI スレッドで動く。~~ (2026-09-29): スクリプトはパスを UTF-8 バイトの 16 進で出す(`file_dialog.rs`。本物の PowerShell を通すテストあり。この PC でも CP932 で出ることを確認した)。ダイアログは別スレッドで開き、閉じるまでボタンを無効にする。`-NoProfile -STA` で起動する。ダイアログの見た目は未確認(ウィンドウの後ろに出る可能性は以前と同じ)。
- ~~**P1: インストーラは Steam のルートにある SteamVR しか探さない。** 別ライブラリの場合に案内する `driver/install.bat` は同梱されていない。~~ (2026-09-29): インストーラとアンインストーラは、コンパニオンの `--register-driver` / `--unregister-driver` を呼ぶ。コンパニオンは `openvrpaths.vrpath` の `runtime` と、`libraryfolders.vdf` にあるすべての Steam ライブラリから SteamVR を探して、その vrpathreg を実行する(終了コード 0 / 2 = SteamVR なし / 3 = vrpathreg 失敗)。見つからなければ「SteamVR を一度起動してからインストーラを再実行」と案内する。コンパニオンの Install Driver も同じ探し方になった(以前はレジストリの Steam フォルダに SteamVR がないと、ほかを探さなかった)。探す処理はテストあり。NSIS は CI でビルドするだけで、実際のインストールは未確認。
- **P1: 署名。** リリースジョブは署名なしでも公開する。ドライバ DLL とアンインストーラは署名しない。Android の keystore が未設定だと毎回一時鍵で署名され、`adb install -r` が更新に失敗する。
- P2: スライダーが範囲外の正しい値を丸めて保存する（sleep 30–900 など）。保存のたびに全キーを書く。保存失敗を再試行しない。`adb devices` を 3 秒ごとに UI スレッドで実行する。シミュレーション停止の join が UI スレッドで最大 5 秒。Deploy が全 adb デバイス（スマホも）に入れ、タイムアウトがない。デモモードでも Install / Uninstall が動く。「Install Driver」は作業ディレクトリ相対で、管理者権限が要る。`build.bat` の `%ERRORLEVEL%` がブロック内で展開される。fuzz / long-run / coverage が continue-on-error。SteamVR 実行中の上書きインストールを確認しない。DESIGN.md との差（Instrument Serif 未使用、text-muted の色、型スケール外のサイズ、ステータスドットの点滅なし）。
- ~~P2（CI）: `SessionE2E.PairsAndStreamsVideoFromTheRealEngine` が一度、失われたフレーム 4 (上限 2) で落ちた~~ (2026-09-30: #43 で 26 で落ちて原因が分かった。クライアントの `FrameAssembler` がタイムアウトをフレームの最初のパケットから数えていたので、受信スレッドがフレームの途中で 100 ms 止まると、残りのパケットがソケットに届いているのにフレームを捨て、次のキーフレームまで映像を止めていた。HMD でも起きうる。最後のパケットから数えるように直した。host テスト `AReceiverPausedMidFrameKeepsTheFrame`) 以下は当時の記録:(2026-09-29、PR #22。ドライバだけの変更で、再実行で通った)。同じ PC 内の UDP でも、CI ランナーが混むと落とす。続くようなら上限か受信バッファを見直す。(2026-09-29: #33 でも 12 で落ちた。受信バッファはもともと 4 MB で、ランナーの一時停止で組み立てのタイムアウト(100 ms)に当たったものと見られる。上限を受け取ったフレームの 1 割にした)
- P2（ドキュメント）: TROUBLESHOOTING が存在しない UI を案内している（「Reinstall driver」、`logs\engine.log`、ビットレートグラフ）。FAQ の「約 30% の帯域削減」は NVENC が動いていないので実測できていない。(2026-09-28: USER_GUIDE の APK のドラッグ&ドロップ・「Deploy」ボタン・HMD での PIN 入力の記述は、実際の UI に合わせて直した)

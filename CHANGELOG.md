# Changelog

All notable changes to Focus Vision PCVR will be documented in this file.

## [0.1.0.0] - 2026-03-27

### Added
- **Video pipeline Phase 1 (PC):** NVENCエンコーダー (テストパターンモード付き)、D3D11テクスチャコピー、DirectMode統合
- **Video pipeline Phase 2 (Android):** MediaCodecデコード (Surface出力対応)、OpenGL ESレンダリング (external OESシェーダー)、UDP受信パイプライン
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

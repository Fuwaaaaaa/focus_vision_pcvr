# TODOS

## v1.0 スコープ内

### FecEncoderのフレームループ外再利用
- **What:** `pipeline.rs`の`encode_frame_to_packets()`で毎フレーム`FecEncoder::new()`を呼んでいるのを、ループ外で一度だけ作成して再利用する
- **Why:** Reed-Solomon初期化にGalois Fieldテーブル計算が含まれ、90fps(11ms/frame)で毎フレーム実行は無駄。累積で0.1ms以上のオーバーヘッド
- **Context:** Eng Review #7で決定。FecEncoderをStreamingEngineまたはrun_streaming()のスコープで保持し、encode_frame_to_packets()に参照渡し
- **Depends on:** ビデオパイプライン実装と同時に対応可能

### H.265 vs H.264 デコードレイテンシー比較調査
- **What:** Focus Vision実機でMediaCodecのH.265とH.264デコードレイテンシーを計測・比較する
- **Why:** Outside Voiceの指摘: H.264はMediaCodecデコードが2-5ms速い可能性。80Mbpsでは画質差が小さく、レイテンシー目標50msに対して2-5msの差は大きい
- **Context:** config/default.tomlのcodecフィールドで切替可能にし、実測値でどちらを採用するか決定。NVENC側はH.264/H.265両対応が容易
- **Depends on:** Phase 2 (Android側デコード) 実装後に計測可能

## v1.1 準備調査

### オーディオパイプラインの仮想オーディオデバイス調査
- **What:** Windowsでの仮想オーディオデバイス作成方法を調査
- **Why:** v1.1のオーディオパイプラインはWASAPI仮想オーディオデバイスを前提としているが、カーネルモードドライバーが必要で巨大なスコープになる可能性
- **Context:** 調査対象: (1) ALVRのalvr_audio (2) WASAPI loopback capture (3) VB-CABLE依存
- **Depends on:** なし（v1.0と並行調査可能）

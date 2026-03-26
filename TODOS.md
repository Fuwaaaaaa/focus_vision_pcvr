# TODOS

## v1.0 スコープ内

### PIN ペアリングのレート制限追加
- **What:** 4桁PINペアリングに3回失敗で60秒ロックを実装
- **Why:** レート制限なしの4桁PIN（10,000通り）は数秒でブルートフォース可能
- **Context:** ペアリング処理はRust Streaming Engineの接続管理モジュールで実装予定
- **Depends on:** TCP制御チャンネルの基本実装

## v1.1 準備調査

### オーディオパイプラインの仮想オーディオデバイス調査
- **What:** Windowsでの仮想オーディオデバイス作成方法を調査
- **Why:** v1.1のオーディオパイプラインはWASAPI仮想オーディオデバイスを前提としているが、カーネルモードドライバーが必要で巨大なスコープになる可能性
- **Context:** 調査対象: (1) ALVRのalvr_audio (2) WASAPI loopback capture (3) VB-CABLE依存
- **Depends on:** なし（v1.0と並行調査可能）

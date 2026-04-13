# 簡易遅延ベース帯域推定 (GCC Lite)

**Date:** 2026-04-13
**Goal:** Transport Feedbackの到着タイムスタンプから遅延勾配を計算し、ロスが発生する前にネットワーク混雑を検出してビットレートを調整する。

---

## 背景

現行の`BandwidthEstimator`はパケットロス率のEWMAのみで帯域推定している。ロスベースの問題点は、パケットが実際に失われた後にしか反応できないこと。Wi-Fi環境ではバッファが溜まりレイテンシーが増加してからロスが発生するため、遅延ベースの検出で先手を打てる。

v2.2でTransport Feedback（パケットごとの受信タイムスタンプ差分）をプロトコルに追加済みだが、データは受信するだけで未使用（`engine.rs:420` TODO）。

## アルゴリズム

### One-Way Delay Gradient

Transport Feedbackの `recv_delta_us`（パケット間の到着間隔µs）の変化を追跡する。

```
gradient[i] = recv_delta[i] - recv_delta[i-1]

gradient > 0 → 到着間隔が伸びている → キューイング遅延増加 → 混雑
gradient < 0 → 到着間隔が縮んでいる → 混雑解消
gradient ≈ 0 → 安定
```

EWMAで平滑化: `delay_gradient = α * gradient + (1-α) * delay_gradient` (α=0.3)

### 状態判定

| 条件 | 状態 | アクション |
|------|------|-----------|
| `delay_gradient > 2.0ms` | OVERUSE | ビットレート -10% |
| `delay_gradient < -1.0ms` かつ loss < 1% | UNDERUSE | 増速許可（ヒステリシス後） |
| それ以外 | NORMAL | 維持 |

### ロスベースとの統合

BitrateControllerの`adjust()`で遅延判定を先に実行し、その後にロスベースの既存ロジックを適用する。両方が減速を示す場合、より大きい減速幅を採用する（max reduction）。

```
adjust():
  1. 遅延ベース: gradient > 2.0 → candidate = -10%
  2. ロスベース: loss > 5% → candidate = -20%, loss > 2% → candidate = -5%
  3. reduction = max(delay_reduction, loss_reduction)
  4. 増速: gradient < -1.0 AND loss < 1% AND hysteresis OK → +5%
```

## 変更ファイル

| File | Action | 内容 |
|------|--------|------|
| `adaptive/bandwidth_estimator.rs` | Modify | `process_feedback()`, `delay_gradient()` 追加 |
| `adaptive/bitrate_controller.rs` | Modify | `adjust()` に遅延ベース判定追加 |
| `engine.rs` | Modify | TODO行を `bw_estimator.process_feedback(&entries)` に置換 |

## BandwidthEstimator 変更詳細

新フィールド:
- `delay_gradient_ms: f64` — 遅延勾配のEWMA (ms単位)
- `last_recv_delta_us: Option<i32>` — 前回の受信間隔

新メソッド:
```rust
/// Transport Feedbackエントリから遅延勾配を更新
pub fn process_feedback(&mut self, entries: &[TransportFeedbackEntry])

/// 現在の遅延勾配 (ms)。正=混雑、負=回復
pub fn delay_gradient(&self) -> f64
```

`process_feedback()` の実装:
1. entriesが2個未満なら何もしない
2. 各 `entries[i].recv_delta_us - entries[i-1].recv_delta_us` を計算
3. µs→msに変換してEWMAで`delay_gradient_ms`を更新
4. `last_recv_delta_us`を最後のエントリで更新

## BitrateController 変更詳細

`adjust()` の変更:
- 既存の `estimator.loss_rate()` に加えて `estimator.delay_gradient()` を参照
- gradient > 2.0 → -10% 減速（ロスが0%でも）
- gradient < -1.0 かつ既存の増速条件を満たす → 増速を許可

## テスト

| テスト | 内容 |
|--------|------|
| `test_process_feedback_stable` | 等間隔到着 → gradient ≈ 0 |
| `test_process_feedback_congestion` | 増加する到着間隔 → gradient > 0 |
| `test_process_feedback_recovery` | 減少する到着間隔 → gradient < 0 |
| `test_process_feedback_empty` | 空/1エントリ → パニックなし |
| `test_process_feedback_single` | 1エントリのみ → gradient変化なし |
| `test_adjust_overuse_without_loss` | ロス0%でもgradient高 → ビットレート低下 |
| `test_adjust_delay_and_loss_combined` | 両方減速 → 大きい方を採用 |

## スコープ外

- カルマンフィルタ（フルGCC）
- Bandwidth probing（安定時にバーストで上限を探索）
- Burst detector（一時的Wi-Fi干渉の分類）

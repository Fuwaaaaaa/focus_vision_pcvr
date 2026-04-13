# cargo-fuzz + arbitrary 構造化Fuzzing導入

**Date:** 2026-04-13
**Goal:** FEC/RTP/プロトコル/設定パーサーに対するfuzzingターゲットを追加し、信頼できないネットワーク入力によるクラッシュ・メモリ安全性違反を自動検出する。

---

## 背景

v2.2の2回のコードレビューで合計17件の問題が見つかった。うち5件はネットワーク入力のパース/検証に関する問題（FECゼロ長シャード、シャードサイズ不一致、オーバーフロー等）。人間のレビューでは限界があるため、libFuzzerによる自動検出を導入する。

## アーキテクチャ

```
rust/streaming-engine/fuzz/
├── Cargo.toml
└── fuzz_targets/
    ├── fuzz_rtp.rs        # RTPパケタイズ→デパケタイズ往復
    ├── fuzz_fec.rs        # FECエンコード→欠損→デコード
    ├── fuzz_protocol.rs   # TCP制御メッセージパーサー
    └── fuzz_config.rs     # TOMLパース+バリデーション
```

## Fuzzingターゲット

### 1. fuzz_rtp — RTP往復テスト

- **入力:** `Arbitrary` で `(frame_data: Vec<u8>, frame_index: u32, is_idr: bool)` を生成
- **処理:** packetize() → 各パケットのヘッダー整合性チェック
- **検証:** パニック・OOM・無限ループが起きないこと
- **max_len:** 65536 (最大フレームサイズ相当)

### 2. fuzz_fec — FEC往復テスト

- **入力:** `Arbitrary` で `(data: Vec<u8>, redundancy: u8[1-100], drop_mask: Vec<bool>)` を生成
- **処理:** encode() → drop_maskに従いシャード欠損 → decode()
- **検証:**
  - 復元可能ケース: decode結果が元データと一致
  - 復元不可能ケース: Errを返しパニックしない
- **max_len:** 4096

### 3. fuzz_protocol — プロトコルパーサー

- **入力:** 生バイト列 `&[u8]` をそのまま渡す
- **処理:** `parse_transport_feedback()`, `parse_hello_version()`, `encode/decode_transport_feedback` 往復
- **検証:** 不正入力でパニックしないこと
- **max_len:** 1024

### 4. fuzz_config — 設定バリデーション

- **入力:** `Arbitrary` で生成した `String` をTOMLとしてパース
- **処理:** `toml::from_str::<AppConfig>()` → `validate()`
- **検証:** 不正TOML・極端値・NaN・空文字列でパニックしないこと
- **max_len:** 4096

## 依存関係

`fuzz/Cargo.toml`:
- `libfuzzer-sys = "0.4"`
- `arbitrary = { version = "1", features = ["derive"] }`
- `streaming-engine` (path = "..")
- `fvp-common` (path = "../../common")

## 実行方法

```bash
# 個別ターゲット (5分間)
cargo fuzz run fuzz_rtp -- -max_len=65536 -max_total_time=300
cargo fuzz run fuzz_fec -- -max_len=4096 -max_total_time=300
cargo fuzz run fuzz_protocol -- -max_len=1024 -max_total_time=300
cargo fuzz run fuzz_config -- -max_len=4096 -max_total_time=300

# 全ターゲット一括
for t in fuzz_rtp fuzz_fec fuzz_protocol fuzz_config; do
  cargo fuzz run $t -- -max_len=65536 -max_total_time=60
done
```

クラッシュが見つかったら `fuzz/artifacts/<target>/` に再現入力が保存される。

## スコープ外

- CI統合 (GitHub Actions でのfuzzing自動実行)
- C++クライアント側のfuzzing
- OSS-Fuzz統合

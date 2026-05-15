# Focus Vision PCVR v3.0.0-rc2

**リリース日:** 2026-05-15
**ステータス:** Release Candidate（実機検証前 / rc1 + 3 修正）
**rc1 からの差分:** PR #59 / #60 / #61（commits `eaace7a` / `a3de671` / `a9fadd7`）

v3.0.0-rc1（2026-05-14）のメンテナンス候補です。rc1 公開後に未取り込み
だった 2026-04-24 コードレビュー指摘の 3 修正を、新しいブランチで起票し
直して main に取り込んだものです。**新機能はゼロ**、**実機要件もゼロ**で、
品質と安全性の純増のみ。

---

## ハイライト

### Security — TLS TOFU ピン留めとフォールバック撤去 (#61)
Android クライアント `TcpControlClient` がこれまで `MBEDTLS_SSL_VERIFY_NONE`
でサーバ証明書を**一切検証していなかった**ため、SECURITY.md が宣言していた
TOFU 緩和策が実態として動作しておらず、同一 LAN 上の攻撃者が任意 TLS 証明書で
MITM を成立させ PIN を盗聴可能でした。本リリースでは:

- TLS ハンドシェイク後にサーバ leaf cert の SHA-256 を計算し、
  `<app internal storage>/server_fingerprint.hex` に永続化
- 以降の接続では fingerprint 一致を強制し、不一致なら接続を拒否
- 比較は同長 hex 文字列に対する byte XOR の OR 累積（best-effort 定数時間）
- 再ペアリングが必要なときは fingerprint ファイルを削除（手順をログに明記）
- TLS / pinning が失敗した場合の**平文フォールバックも削除**

設計と監査メモは `SECURITY.md` の更新と PR #61 を参照。

> **GA 前必須:** Android 実機 / エミュレータ + mitmproxy 等での 5 項目
> 動作確認（PR #61 の test plan セクション参照）。コードのみリリースに
> 含まれており、real-device verification は未完です。

### Fix — NVENC セッションリーク 2 経路を塞ぐ (#59)
GeForce は同時 NVENC セッションを 2 本までに制限しており、pair / unpair を
繰り返すと leak が積み上がって次のユーザーで `NV_ENC_ERR_OUT_OF_MEMORY` が
出る既知経路を塞ぎました:

1. **再 init**: `NvencEncoder::init()` が `m_initialized` のとき早期 return
   していたパスを `shutdown()` 経由に変更。reconfigure 時のセッションリーク
   を防ぐ。
2. **partial-init fallback**: `loadNvencApi()` 成功後に
   `createEncoderSession()` / `createResources()` で失敗した場合、`m_encoder`
   だけがクリーンアップされ DLL + buffer + registered resource が残存して
   いた。fallback パスも `shutdown()` に統一。
3. `shutdown()` の `m_initialized` ガード撤去。全フィールドが null-checked
   なので partial 状態で呼んでも安全。

ctest 36/36 で回帰なし確認。

### Observability — `spawn_named` ヘルパーで長寿命タスクに名前を付与 (#60)
streaming engine の 4 つの長寿命 tokio spawn を `spawn_named(handle, name, fut)`
経由に集約しました:

- `streaming`
- `tracking-receiver`
- `audio-encoder`
- `tcp-control`

spawn 時に debug log で名前を出力するので、ログストリームが途切れた
ときにどのサブシステムが停止したかをログだけで切り分け可能になります。
workspace は `panic = "abort"` プロファイルなので `catch_unwind` は不要
（doc コメントに `panic = "unwind"` 切替時の拡張方法を明記）。

注意: 本変更は PR #52 の直接 cherry-pick ではなく**手動で再実装**しました。
旧 PR #52 の base は古すぎて、そのまま取り込むと rc1 で生きている
`RECORDING_ENABLED` / `AUDIO_RECORDING_ENABLED`（CONFIG_UPDATE 0x03 / 0x05）と
`rustls::crypto::ring::default_provider().install_default()` を消す破壊的差分が
混入していたためです。今回の #60 はこれら 3 つを保全しています。

---

## Provenance — なぜ「rc1.1」ではなく rc2 なのか

3 修正は 2026-04-24 のコードレビューで提起され、各々独立した PR
（#51 NVENC / #52 spawn_named / #53 TLS）として存在していました。
rc1 cut の時点では 3 PR とも main から大きく diverge していて、GitHub UI
の rebase ボタンでは取り込めない状態でした:

- #51 → `c47c653` を fresh branch `fix/nvenc-session-lifecycle-v2` 上で cherry-pick → #59 → squash merge
- #52 → 旧 base 古過ぎのため**手動再実装** → #60 → squash merge
- #53 → `b0f96a4` を fresh branch `security/client-tls-tofu-pinning-v2` 上で cherry-pick（CHANGELOG.md / SECURITY.md の文章 conflict を `### Security` 挿入で resolve）→ #61 → squash merge

3 commit はいずれも CI 全 ジョブ green で main に取り込み済みです。

---

## インストール

rc1 と完全同じ手順です。署名パイプライン・NSIS installer・APK keystore も
変化なし。

### Windows (companion + driver)
1. [Releases](https://github.com/Fuwaaaaaa/focus_vision_pcvr/releases/tag/v3.0.0-rc2) から
   `FocusVision-v3.0.0-rc2-Setup.exe` をダウンロード
2. 実行し、インストールウィザードに従う
3. SteamVR を起動 → ドライバが自動検出

### Android (Focus Vision HMD)
1. companion app の Deploy タブから APK 選択
2. HMD を USB 接続し ADB デバッグ有効化
3. `Install APK on All Devices`

詳細は `docs/USER_GUIDE.md`。

---

## Breaking changes

なし。rc1 と完全後方互換。`status.json` schema / TCP control message format
変化なし。設定ファイル `local.toml` のフィールドも変わらない。

> 例外: Android クライアントは初回接続時に `server_fingerprint.hex` を内部
> ストレージに作成します。サーバの TLS 証明書を生成し直した場合（通常は
> 起動毎に ephemeral）、最初の接続でピンが捕まり以後の `--rc1`-built
> クライアントは TLS 失敗で接続できなくなります。サーバ側を rc2 に上げる
> 前にクライアント側も rc2 を入れてください。

---

## Known Issues / 実機未検証項目 (rc1 から継続)

- **TLS TOFU 実機検証**: PR #61 の test plan チェックリスト 5 項目
  （初回接続でのピン作成 / 再接続での一致確認 / 証明書変化での拒否 /
  mitmproxy 経由の MITM 拒否 / fingerprint 削除での復旧）
- **DRS (Dynamic Resolution Scaling)** — コード未着手、Phase 4.x
- **NVENC VUI フルレンジ実色彩確認** — 単体テストは ctest 6 件で完了、
  実機目視は GA で
- **NVML 連携実 GPU 温度** — コード完了、閾値妥当性は GA で
- **HTC Face Tracking 実カメラ入力 → OSC end-to-end**

---

## 検証チェックリスト

実機なしで動作確認できる項目（rc1 と同じ + 今回 ctest 件数増）:

```sh
# 1. ビルド
cargo build --release --workspace

# 2. テスト (workspace 全体)
cargo test --workspace

# 3. lint
cargo clippy --workspace -- -D warnings

# 4. Driver C++ テスト (36 件、rc1 と同じ)
cd driver/build && cmake --build . --config Release && ctest --build-config Release

# 5. companion app の手動 smoke test
cargo run -p focus-vision-companion

# 6. Demo モード (実機なしで全 UI を確認)
cargo run -p focus-vision-companion -- --demo

# 7. Headless E2E (TCP+TLS+PIN+RTP+FEC+UDP)
cargo run -p streaming-engine --bin focus-vision-headless --features simulator
```

---

## サポート

- Issues: https://github.com/Fuwaaaaaa/focus_vision_pcvr/issues
- ドキュメント: `docs/USER_GUIDE.md`, `docs/FAQ.md`, `docs/TROUBLESHOOTING.md`
- rc1 リリースノート: `docs/RELEASE_NOTES_v3.0.0-rc1.md`

---

🤖 Generated with [Claude Code](https://claude.com/claude-code)

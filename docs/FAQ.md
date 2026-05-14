# Focus Vision PCVR FAQ

セットアップと操作については [USER_GUIDE.md](USER_GUIDE.md)、トラブル対処は [TROUBLESHOOTING.md](TROUBLESHOOTING.md) を参照してください。ここではより制度的・概念的な質問に答えます。

---

## ライセンス・利用条件

### Q. 商用利用できますか?

A. **デュアルライセンス**です。

| 用途 | ライセンス | 費用 |
|------|-----------|------|
| 個人利用、教育、研究、非商用 OSS | [MIT](../LICENSE) | 無料 |
| 商用利用 (販売、収益化を含む製品/サービス) | Commercial License | 有料 |

商用ライセンスのお問い合わせは [GitHub Issue](https://github.com/Fuwaaaaaa/focus_vision_pcvr/issues) からお願いします。

### Q. 自分の OSS プロジェクトに組み込めますか?

A. 該当 OSS プロジェクトが**非商用**であれば MIT で組み込めます。商用 SaaS や有料アプリへの組み込みには商用ライセンスが必要です。

### Q. フォークして改変したものを配布できますか?

A. MIT の条件下では可能 (著作権表示と免責条項を保持)。改変版に商用利用を含む場合は商用ライセンスが別途必要になります。

---

## ハードウェア互換性

### Q. VIVE Focus Vision 以外の HMD で動きますか?

A. **現時点では非対応**です。

- プロジェクトは Focus Vision 専用に最適化されています (XR_HTC_facial_tracking、HTC VIVE Focus 3 コントローラープロファイル、Vision の eye tracking SDK 依存)
- Quest 系 / Pico 系での動作は未確認、対応予定もありません
- Quest を使いたい場合は ALVR や Virtual Desktop など既存プロジェクトを推奨

### Q. AMD / Intel GPU で動きますか?

A. **非対応**です。NVENC ハードウェアエンコードに依存しているため、AMD VCE / Intel QSV はサポートしません。

将来的にも、最も最適化されている NVENC を中心に保つ方針です。

### Q. 古い NVIDIA GPU (GTX 900 番台、Maxwell) で動きますか?

A. **GTX 1060 6GB 以上を推奨**です。

- Maxwell (900番台) でも NVENC 自体は使えますが H.265 エンコードに対応していないため、`codec = "h264"` 設定が必須
- GTX 1060 未満の VRAM ではフレームバッファ + エンコードバッファの確保が苦しく、80 Mbps 設定で不安定になりがち

---

## 機能・仕様

### Q. H.264 と H.265 (HEVC) はどちらを使うべき?

A. **HMD 側 MediaCodec のデコード性能による**ので、初回接続時に両方を 5秒ずつベンチマークして低レイテンシな方を自動選択します (`v1.1.0` 以降)。

| | H.264 | H.265 (HEVC) |
|---|---|---|
| 帯域効率 | △ | ◎ (同画質で 30-50% 削減) |
| デコードレイテンシ | ◎ | △ (一部 SoC で 2-5ms 遅い) |
| HMD 対応 | 全機種 | Focus Vision OK |
| 既定 | — | ✓ |

迷ったら **H.265 を選んでください**。差が分かるほどに違う場合は自動選択ロジックが H.264 に切り替えます。

### Q. 帯域はどれくらい必要?

A. **80 Mbps をリファレンスに**しています。

- 80 Mbps H.265 で 1832×1920 @ 90fps = Focus Vision 本来の解像度・リフレッシュレート
- Wi-Fi 5 (5GHz) なら理論上問題なし、Wi-Fi 6 ならマージンあり
- 帯域不足を検知すると Adaptive Bitrate が自動で下げます (最低 8 Mbps の sleep_bitrate まで)
- FEC が常時 5-40% 付加されるので実効帯域は表示値の 1.05-1.40 倍

### Q. ワイヤードで動かせますか?

A. プロトコルは TCP + UDP なので**論理的には可能**ですが、現在のセットアップフローは Wi-Fi 経由前提で設計されています。USB テザリングや USB-C → Ethernet で接続する場合も Wi-Fi 同様の TCP/UDP 通信になるので、ファイアウォール設定さえ揃っていれば動くはずです。

### Q. Foveated Encoding は何 % 帯域を削減しますか?

A. プリセットによります:

| プリセット | fovea QP offset | peripheral QP offset | 帯域削減 |
|---------|---|---|---|
| subtle | +3 | +8 | ~10% |
| balanced (既定) | +5 | +15 | ~20% |
| aggressive | +8 | +25 | ~30% |
| custom | 任意 | 任意 | 設定次第 |

視線追従 (XR_EXT_eye_gaze_interaction) を使うため、Eye Tracking が無効化された状態では中央固定のフォビーション (画面中心が高画質) になります。

### Q. NVENC ROI は使えますか?

A. **v3.0 では非対応**。実機 (NVENC SDK 12.x 対応 GPU + Focus Vision) で検証できる環境が揃った時点で再着手予定です ([TODOS.md](../TODOS.md))。現状の QP delta map で ~30% 帯域削減を達成しており、実用上の差は小さい範囲です。

---

## セキュリティ・プライバシー

### Q. TCP 通信は暗号化されていますか?

A. **TLS 1.3 で暗号化済み** (rustls / MbedTLS)。詳細は [SECURITY.md](../SECURITY.md) 参照。

### Q. UDP の映像/音声/トラッキングデータは暗号化されていますか?

A. **暗号化されていません**。

- Wi-Fi WPA2/WPA3 暗号化に依存
- 機微情報を扱うアプリ (画面共有でパスワード入力など) には現状不向き
- UDP 暗号化は将来的に検討中 ([SECURITY.md](../SECURITY.md) Known Limitations 参照)

### Q. Session Recording で何が録画されますか?

A. オプション機能で、有効時のみ以下を `%APPDATA%\FocusVisionPCVR\recordings\` に保存:

- 映像: Annex B raw NAL (`recording_<timestamp>.h265` または `.h264`)
- 音声: 16-bit PCM WAV (`recording_<timestamp>.wav`)

既定では **無効** ([USER_GUIDE.md](USER_GUIDE.md) 「設定のカスタマイズ」参照)。`config/local.toml` で `[recording] enabled = true` にして初めて録画開始します。

録画ファイルは `retention_days` (既定 30日) で自動削除。0 にすると無限に保持。

### Q. PIN 認証は強いですか?

A. **6桁数字 PIN + 5回ロックアウト + 300秒クールダウン + TLS 1.3** の組み合わせで、1秒で 5回試行できる攻撃者でも平均突破に数日要する計算 (1,000,000 ÷ 2 ÷ 5 試行 = ロックアウト 100,000 回 = ~347日)。攻撃の現実性は低い設定です。

ローカル Wi-Fi 環境前提なので、リモート攻撃には WAN ファイアウォールが先に立ちはだかります。

### Q. テレメトリ送信はありますか?

A. **送信ゼロ**です。

- 自動収集ログなし
- Phone-home なし
- Export Logs ボタンは「ZIP 作成して保存先を聞く」ローカル動作のみ

---

## 比較・他プロジェクト

### Q. VIVE Hub と何が違うのですか?

A. 重複機能と差別化機能があります:

| 機能 | VIVE Hub | Focus Vision PCVR |
|------|---------|---|
| ワイヤレス DP モード | ◎ (公式) | △ (本プロジェクトは Wi-Fi のみ) |
| ハンドトラッキング | ◎ | × (未対応) |
| パススルー | ◎ | × (未対応) |
| Face Tracking → VRChat | × | **◎ (本プロジェクトの強み)** |
| Foveated Encoding | △ | **◎** |
| レイテンシー最適化 | △ | **◎ (適応ビットレート + FEC + GCC)** |
| Session Recording | × | **◎** |
| オープンソース | × | **◎ (MIT デュアル)** |
| カスタマイズ | 限定的 | **◎ (config.toml + ソース改変可)** |

特に **VRChat で表情を使いたい人**、**Wi-Fi ネットワークを最適化して低遅延を追求したい人**、**録画でセッションを残したい人** に向いています。

### Q. ALVR / Virtual Desktop と何が違いますか?

A. ALVR / Virtual Desktop は Meta Quest / Pico をターゲットにした成熟プロジェクトです。Focus Vision 専用にする利点は:

- HTC のフェイストラッキング SDK にネイティブ対応
- 設定がシンプル (Quest 系では多数の HMD バリエーションに対応する設定が必要)
- 1機種に最適化することで余計な抽象化レイヤを減らせる

逆に複数機種 (Quest + Focus 等) を持っているなら ALVR 系が便利です。

---

## 開発・コントリビューション

### Q. ソースからビルドできますか?

A. はい。Windows + Rust stable + CMake + Android NDK r26 があれば `./build.bat` 一発でビルドできます。詳細は [CONTRIBUTING.md](../CONTRIBUTING.md) 参照。

### Q. プラグインを書いて機能拡張できますか?

A. **v3.0 では正式 API 未定**ですが、v3.0 以降にコミュニティプラグイン API を計画中 ([TODOS.md](../TODOS.md))。

それまでは config.toml と env var で挙動を変更する範囲内になります。

### Q. バグ報告 / 機能要望はどこへ?

A. [GitHub Issues](https://github.com/Fuwaaaaaa/focus_vision_pcvr/issues) でお願いします。バグ報告には [TROUBLESHOOTING.md](TROUBLESHOOTING.md) の「ログを集める」で生成した ZIP を添付していただくと迅速に対応できます。

---

## 関連ドキュメント

- [USER_GUIDE.md](USER_GUIDE.md) — セットアップガイド
- [TROUBLESHOOTING.md](TROUBLESHOOTING.md) — トラブル対処
- [ARCHITECTURE.md](../ARCHITECTURE.md) — 技術構成
- [SECURITY.md](../SECURITY.md) — 脅威モデル詳細
- [TODOS.md](../TODOS.md) — 将来計画
- [LICENSE](../LICENSE) — MIT 全文

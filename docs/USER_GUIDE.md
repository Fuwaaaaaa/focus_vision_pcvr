# Focus Vision PCVR ユーザーガイド

VIVE Focus Vision を Wi-Fi 経由で PC につなぎ、SteamVR ゲームをワイヤレスでプレイするためのセットアップガイドです。

技術的な説明より「動かすまでの最短手順」を重視しています。詳しい仕様は [`ARCHITECTURE.md`](../ARCHITECTURE.md)、トラブル対処は [`TROUBLESHOOTING.md`](TROUBLESHOOTING.md)、よくある質問は [`FAQ.md`](FAQ.md) を参照してください。

---

## 1. 必要なもの

| カテゴリ | 要件 | 備考 |
|---------|------|------|
| **PC** | Windows 10 (1909+) / Windows 11 | 32-bit Windows は非対応 |
| **GPU** | NVIDIA GeForce GTX 1060 6GB 以上 | NVENC ハードウェアエンコード必須。AMD / Intel GPU は非対応 |
| **GPU ドライバ** | NVIDIA Game Ready Driver 528 以上 | 古いドライバは NVENC 構造体が異なり起動に失敗します |
| **SteamVR** | 最新安定版 | Steam クライアントから自動更新 |
| **HMD** | VIVE Focus Vision | Quest など他機種は非対応 |
| **Wi-Fi** | Wi-Fi 5 (5GHz) 以上、PC と HMD が同じネットワーク | Wi-Fi 6E 推奨。2.4GHz は遅延が大きすぎて非推奨 |
| **USB ケーブル** | データ転送対応 USB-A → USB-C | APK 配信用。配信完了後は外して構いません |

> **ヒント:** ルーターと PC を有線で接続すると、Wi-Fi の干渉が PC ↔ ルーター区間でゼロになるため映像品質が大きく改善します。HMD ↔ ルーター区間のみ Wi-Fi を使う構成が最強です。

---

## 2. 5ステップ・セットアップ

### ステップ 1: インストーラをダウンロード

1. [GitHub Releases](https://github.com/Fuwaaaaaa/focus_vision_pcvr/releases/latest) を開く
2. **Assets** から `FocusVision-3.0.0-Setup.exe` を選んでダウンロード
3. (オプション) APK を Play Store で配信していないため、HMD 側アプリも同じページから `FocusVision-Client-3.0.0.apk` をダウンロード

> **SmartScreen 警告について:** Authenticode 証明書を取得するまでの間、初回起動時に Windows Defender SmartScreen が「認識されないアプリです」と警告を出します。**詳細情報 → 実行** で進めれば動作します。信頼性が気になる場合は、リポジトリのソースコードから自分でビルドできます (`./build.bat`)。

### ステップ 2: PC コンパニオンアプリをインストール

ダウンロードした `FocusVision-3.0.0-Setup.exe` をダブルクリックし、ウィザードに従ってインストールします。デフォルトインストール先: `C:\Program Files\Focus Vision PCVR\`。

インストール時に **SteamVR ドライバの自動登録** も行われるので、追加で `vrpathreg adddriver` を手動実行する必要はありません。

インストール完了後、デスクトップに「Focus Vision PCVR」ショートカットができます。

### ステップ 3: コンパニオンアプリを起動 + 初回設定

ショートカットから起動すると、ホームタブが表示されます:

- **Driver status**: `Installed ✓` であることを確認
- **PIN**: `------` (まだ表示されない) — HMD 接続待ち

次に **Deploy to HMD タブ** に移動して APK をインストールします。

### ステップ 4: HMD に APK を配信

1. VIVE Focus Vision の **開発者モード** を有効化 (設定 → システム → ビルド番号を7回タップ → 開発者オプション → USB デバッグ ON)
2. USB-C ケーブルで HMD を PC に接続
3. HMD 側で「USB デバッグを許可しますか?」が表示されたら **常に許可** を選択
4. コンパニオンアプリの **Deploy タブ** で:
   - **Refresh** ボタンを押すと検出されたデバイスが表示される
   - **APK パス** に手順1でダウンロードした `FocusVision-Client-3.0.0.apk` をドラッグ&ドロップ
   - **Deploy** ボタンを押す
5. 「Install complete」と表示されたら USB ケーブルを外して OK

### ステップ 5: HMD で接続

1. HMD 側で、ライブラリ → 「Focus Vision PCVR」を起動
2. PC 側コンパニオンアプリの **Home タブ** に **6桁の PIN** が表示される (例: `123456`)
3. HMD 画面に表示される PIN 入力欄に PC 側の PIN を入力
4. PC 側に **Connected ✓** と表示されたら成功。SteamVR ゲームをプレイできます

> **PIN は接続のたびに変わります** — セキュリティのため、エンジン再起動 + 各セッションで新規生成されます。

---

## 3. 接続できたら確認すること

最初のセッションでは以下を確認しておくと、後のトラブル対処が早くなります。

### レイテンシーグラフ (Home タブ)

接続後、Home タブにレイテンシー / FPS / パケットロスのリアルタイムグラフが表示されます。

| 指標 | 良好 | 注意 | 危険 |
|------|------|------|------|
| **Latency** | < 40ms | 40-80ms | > 80ms (酔いの原因) |
| **FPS** | 90fps 安定 | 80-90fps で揺れる | 80fps 未満 |
| **Packet loss** | < 1% | 1-5% | > 5% (Wi-Fi 環境見直し) |

レイテンシーが安定しない場合は [`TROUBLESHOOTING.md`](TROUBLESHOOTING.md) の「映像がカクつく」を参照してください。

### HMD 内ステータスオーバーレイ

VR ゲームをプレイ中、視野の右上に小さく **Wi-Fi 信号強度風の3バー** が表示されます。
- 緑 = 良好
- 黄 = 注意
- 赤 = 不安定

ゲーム中にカクつきを感じたら、まずこのバーを見て Wi-Fi 問題か PC 側問題かを切り分けてください。

---

## 4. 日常的な使い方

### 起動順序の推奨

1. PC 側: コンパニオンアプリ起動 → SteamVR 起動
2. HMD 側: VR を被って Focus Vision アプリ起動
3. PIN 入力 → 接続
4. SteamVR ホームから好きなゲームを起動

### 終了

HMD 側のシステムメニューから VR セッションを終了するか、PC 側コンパニオンアプリを閉じれば自動的に切断されます。コンパニオンアプリを閉じても SteamVR ドライバ自体は残るので、次回はアプリを起動するだけで再接続できます。

### Wi-Fi の一瞬切断

Wi-Fi が瞬断しても **5秒の hold ウィンドウ** が設けられているので、自動的に再接続を試みます。音声・録画は hold 中も継続します。

長時間の切断 (5秒超) 時は新しい PIN での再接続が必要になります。

---

## 5. 設定のカスタマイズ

### コンパニオンアプリの Settings タブ

GUI から変更できる主な項目:

- **Video codec**: H.264 / H.265 切替 (H.265 が高画質・帯域効率良。古い HMD では H.264 がデコード低レイテンシ)
- **Sleep mode**: 非活動時に自動的にビットレートを下げる省電力モード
- **Face Tracking**: HTC blendshapes を VRChat OSC に送信 (有効/無効)
- **Session Recording**: ストリーミング中の映像/音声をディスクに自動保存

### config/local.toml で上級設定

`%APPDATA%\FocusVisionPCVR\config\local.toml` を作成すると、より詳細なパラメータを変更できます。テンプレートは `%PROGRAMFILES%\Focus Vision PCVR\config\default.toml` を参考にしてください。

代表的な調整項目:
- `[video] bitrate_mbps`: 既定 80 Mbps。Wi-Fi 環境が悪い場合は 40-60 Mbps に下げると安定
- `[network] congestion_control = "loss"`: GCC delay-based 推定を無効化、ロスベースのみで動作
- `[recording] retention_days`: 録画ファイルの自動削除日数 (既定 30、0 = 削除しない)
- `[thermal] enabled = true`: GPU 温度監視を有効化 (要 `nvml` feature build)

---

## 6. アンインストール

スタートメニュー → 「Focus Vision PCVR」→ **Uninstall** を実行すると、SteamVR からのドライバ登録解除 + プログラムファイル削除 + ショートカット削除がまとめて行われます。

設定ファイル (`%APPDATA%\FocusVisionPCVR\`) と録画ファイル (`%APPDATA%\FocusVisionPCVR\recordings\`) はアンインストールでは削除されません。完全に消すには手動で削除してください。

---

## 関連ドキュメント

- [TROUBLESHOOTING.md](TROUBLESHOOTING.md) — トラブル症状別の対処
- [FAQ.md](FAQ.md) — よくある質問
- [ARCHITECTURE.md](../ARCHITECTURE.md) — システム構成とデータフロー (技術者向け)
- [CONFIG.md](CONFIG.md) — 全 config パラメータリファレンス
- [SECURITY.md](../SECURITY.md) — 脅威モデル、TLS、PIN 認証

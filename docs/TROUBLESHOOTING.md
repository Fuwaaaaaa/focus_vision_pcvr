# Focus Vision PCVR トラブルシューティング

症状から逆引きする対処マニュアル。各セクションは **症状 → 原因候補 → 対処** の流れで構成されています。最初は順番に試してみて、解決しない場合は「ログを集める」へ進んでください。

[USER_GUIDE.md](USER_GUIDE.md) のセットアップを完了している前提です。

---

## 接続関連

### HMD に PC が見つからない / アプリ起動後に PIN 入力画面で止まる

**原因候補と対処:**

1. **PC と HMD が違うネットワークにいる**
   - PC のネットワーク設定で接続中の SSID を確認
   - HMD 側でも 設定 → Wi-Fi で同じ SSID に繋がっているか確認
   - **対処:** 両方を同じ 5GHz ネットワークに繋ぎ直す

2. **Windows ファイアウォールが TCP 9944 / UDP 9945-9948 をブロック**
   - 初回起動時のファイアウォール許可ダイアログで「キャンセル」を押した場合に起こる
   - **対処:** Windows Defender ファイアウォール → 詳細設定 → 受信の規則 → 新しい規則 → ポート → TCP 9944 / UDP 9945-9948 を許可
   - **簡易対処:** コンパニオンアプリを管理者として再起動し、ダイアログで「許可」を押す

3. **ルーターのクライアント分離 (AP isolation) が有効**
   - 公衆 Wi-Fi やホテル Wi-Fi で多い設定。同一ネットワーク内のデバイス同士の通信をブロックする
   - **対処:** ホーム Wi-Fi を使う、もしくはルーター設定で AP isolation を OFF

### PIN 入力したが拒否される (PIN rejected)

**原因候補と対処:**

1. **PIN を入力する直前に PC 側が新しい PIN を生成した**
   - エンジン再起動 / TCP 切断後の再接続のたびに PIN は変わる
   - **対処:** PC 側コンパニオンアプリ Home タブの現在の PIN を再確認して入力し直す

2. **5回連続で間違えた → 300秒ロックアウト**
   - 連続誤入力に対する brute-force 防護機能。300秒待つか、エンジンを再起動して新規 PIN にする
   - **対処:** PC 側コンパニオンアプリを一度終了して再起動。新しい PIN が表示される

### TLS 証明書の警告 / 検証エラー

**原因候補と対処:**

1. **初回接続時の TOFU (Trust On First Use) ピン留め**
   - 初回接続時に PC 側の自己署名証明書を HMD 側にピン留めする方式。2回目以降に異なる証明書が来ると警告する
   - **対処:** PC 側で SteamVR ドライバを再インストールした場合などに発生。HMD 側でアプリのデータを消去 (Settings → Apps → Focus Vision PCVR → Clear Data) して TOFU を再ピン留め

---

## 映像関連

### 映像が出ない / 真っ黒のまま

**原因候補と対処:**

1. **SteamVR がドライバを認識していない**
   - SteamVR ホーム → メニュー → 設定 → 開発者 → 接続の管理 → 「Focus Vision PCVR」が一覧にあり「アクティブ」になっているか確認
   - **対処:** コンパニオンアプリの「Reinstall driver」ボタンを押す → SteamVR を再起動

2. **NVENC エンコーダ初期化失敗**
   - NVIDIA ドライバが古い / 非対応 GPU
   - **対処:** `%APPDATA%\FocusVisionPCVR\logs\engine.log` で `NvencEncoder: nvEncInitializeEncoder failed` の行を確認。NVIDIA Game Ready Driver 528 以降に更新

3. **HMD 側のデコーダエラー**
   - HMD ログ (`adb logcat | grep FocusVision`) に `MediaCodec error` がないか確認
   - **対処:** HMD を再起動。それでも続く場合は codec を H.264 に切替 (Settings タブ)

### 映像がカクつく / フレーム落ちが多い

**原因候補と対処:**

1. **Wi-Fi 信号が弱い**
   - HMD 内ステータスバー (視野右上) が黄〜赤になっていないか
   - **対処:**
     - ルーターを HMD に近づける、もしくは Wi-Fi 中継機を設置
     - 5GHz チャネルを変更 (DFS 帯域を避ける)
     - 2.4GHz 機器 (電子レンジ、Bluetooth) を遠ざける

2. **PC が他の処理で詰まっている**
   - タスクマネージャで GPU 使用率 / CPU 使用率を確認
   - **対処:**
     - GPU が常時 100% → ゲームの画質設定を下げる、ビットレートを下げる (`local.toml` の `bitrate_mbps`)
     - CPU が高い → 不要なバックグラウンドアプリを終了

3. **適応ビットレートが揺れている**
   - 帯域不足を検知して自動的にビットレートを下げている
   - **対処:** Home タブのビットレートグラフを観察。安定して 30 Mbps 未満なら Wi-Fi 環境見直しが本質的解決

4. **GPU が熱で性能低下している (4時間以上連続稼働など)**
   - PC の通気を確認、`config/local.toml` で `[thermal] enabled = true` を設定して thermal governor を有効化 (要 `nvml` feature build)

### 映像にノイズ / 緑のブロック / IDR ストーム

**原因候補と対処:**

1. **パケットロスが多くて FEC で復旧しきれていない**
   - Home タブのパケットロスが 5% 超
   - **対処:** Wi-Fi 環境改善 (上記)、もしくは `config/local.toml` で `fec_redundancy_max = 0.40` を上限まで上げる

2. **スライス FEC の再送タイムアウト連発**
   - Engine log に `slice timeout` 頻発、IDR_REQUEST が 2回/秒で頭打ち
   - **対処:** `slice_count` を下げる (例: 4 → 2)、または `slice_fec_enabled = false` で従来 FEC に戻す

### 映像はあるが視野中央だけ高画質、周辺はモザイク

これは**仕様**です — Foveated Encoding が有効になっています。

- 視線追従で周辺視野を意図的に低画質化して帯域を 30% 削減
- 違和感がある場合は Settings タブで Foveated Encoding を OFF にできる
- もしくは `config/local.toml` で `[foveated] preset = "subtle"` に変更

---

## 音声関連

### 音声が聞こえない

**原因候補と対処:**

1. **PC 側に音声出力デバイスがない / WASAPI loopback 失敗**
   - 例: ヘッドホンを抜いた瞬間に Windows が「再生デバイスなし」状態になる
   - **対処:** PC に音声出力 (内蔵スピーカー、有線/Bluetooth ヘッドホン、HDMI モニター等) を1つ接続。コンパニオンアプリを再起動

2. **HMD 側 AAudio 初期化失敗**
   - **対処:** HMD を再起動

3. **Discord などの仮想オーディオドライバが loopback をブロック**
   - **対処:** PC のサウンド設定で既定の出力デバイスを物理デバイスに戻す

### 音声と映像がずれている

**原因候補と対処:**

1. **PC ↔ HMD のクロック同期ずれ (RTP timestamp の累積誤差)**
   - 数十分の長時間プレイで発生しやすい
   - **対処:** いったん切断して再接続するとリセットされる

2. **Opus エンコーダのフレームサイズ設定が大きすぎる**
   - **対処:** `config/local.toml` で `[audio] frame_size_ms = 10` (既定値) を確認

---

## ハプティクス / トラッキング

### コントローラーが SteamVR に表示されない

**原因候補と対処:**

1. **HMD 側でコントローラーがペアリングされていない**
   - HMD のシステム設定でコントローラー接続状態を確認
   - **対処:** コントローラーを再ペアリング

2. **HTC VIVE Focus 3 コントローラープロファイルが SteamVR に登録されていない**
   - **対処:** SteamVR を最新版に更新。それでも認識されない場合はコンパニオンアプリの「Reinstall driver」

### コントローラーの振動が来ない

**原因候補と対処:**

1. **HAPTIC_EVENT (0x38) パイプラインが詰まっている**
   - `engine.log` で `Haptic dropped: channel full` がないか確認
   - **対処:** 多くの場合は一時的。続く場合はバグ報告を

### 顔表情が VRChat に反映されない (Face Tracking)

**原因候補と対処:**

1. **HMD 側で Face Tracking 権限が付与されていない**
   - HMD 設定 → アプリ → Focus Vision PCVR → 権限 → Face Tracking ON

2. **VRChat OSC ポートが違う**
   - 既定 9000。VRChat 側で別ポートを設定している場合は `config/local.toml` の `[face_tracking] osc_port` を合わせる

3. **アバターに対応ブレンドシェイプがない**
   - **対処:** VRChat 用 FT 対応アバターを選択

---

## SteamVR 関連

### SteamVR が「ドライバが見つかりません」と表示

**原因候補と対処:**

1. **vrpathreg 登録失敗 / 権限不足**
   - **対処:** コンパニオンアプリを管理者として実行 → 「Reinstall driver」

2. **SteamVR の別バージョン (Beta など) を使っている**
   - **対処:** Steam クライアントで SteamVR を安定版に切り替える

### SteamVR がクラッシュする / ハングする

**原因候補と対処:**

1. **Direct Mode 競合 (他の VR ドライバが残っている)**
   - **対処:** SteamVR 設定 → 開発者 → 接続の管理 → 不要な他社製ドライバを無効化

---

## ログを集める

上記で解決しない場合、サポート/Issue 用にログを収集します:

1. PC 側コンパニオンアプリ → Home タブ → **Export Logs** ボタン
2. PC 側ログ + HMD 側 logcat + システム情報 (GPU/ドライバ/Windows バージョン) を ZIP にまとめてくれる
3. PII (IP アドレス、ホスト名) は自動でマスクされる
4. ZIP を [GitHub Issues](https://github.com/Fuwaaaaaa/focus_vision_pcvr/issues) に添付して報告

ログ収集なしで Issue を立てる場合でも、以下の情報があると調査が早いです:

- Windows バージョン (10/11、ビルド番号)
- NVIDIA GPU 型番 + ドライババージョン
- HMD ファームウェアバージョン
- Wi-Fi ルーターの型番 + 5GHz/6GHz 帯域
- いつから起きたか (Wi-Fi 切替後? アプリ更新後? 突然?)
- 再現手順

---

## 関連ドキュメント

- [USER_GUIDE.md](USER_GUIDE.md) — セットアップガイド
- [FAQ.md](FAQ.md) — よくある質問
- [SECURITY.md](../SECURITY.md) — TLS / PIN 関連の詳細
- [ARCHITECTURE.md](../ARCHITECTURE.md) — システム構成 (上級者向け)

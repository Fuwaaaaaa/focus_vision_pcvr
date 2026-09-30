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
   - エンジンは SteamVR の `vrserver.exe` の中で動くので、初めて待ち受けたときに出るファイアウォールの許可ダイアログは vrserver.exe のもの。そこで「キャンセル」を押した場合に起こる (インストーラはファイアウォールの規則を作らない)
   - **対処:** Windows Defender ファイアウォール → 詳細設定 → 受信の規則 → 新しい規則 → ポート → TCP 9944 / UDP 9945-9948 を許可 (または「アプリを許可」で vrserver.exe のプライベート ネットワークを許可)

3. **ルーターのクライアント分離 (AP isolation) が有効**
   - 公衆 Wi-Fi やホテル Wi-Fi で多い設定。同一ネットワーク内のデバイス同士の通信をブロックする
   - **対処:** ホーム Wi-Fi を使う、もしくはルーター設定で AP isolation を OFF

### PIN 入力したが拒否される (PIN rejected)

**原因候補と対処:**

1. **PIN が新しくなった**
   - PIN は発行から 300 秒で新しくなる (Home タブの `Expires in`)。SteamVR の再起動でも変わる。配信中に切れた接続が 5 秒以内に戻るときだけは、そのセッションの PIN がもう一度通る
   - **対処:** PC 側コンパニオンアプリ Home タブの現在の PIN を確認して入力し直す。USB でつないでいれば「Send PIN to headset」で PIN ごとアプリを起動できる

2. **5 回続けて間違えた → 300 秒のロックアウト**
   - 総当たりへの防御。ロックアウトすると PIN も新しくなる。回数と時間は `config/local.toml` の `[pairing]` で変えられる (1〜10 回、300〜3600 秒)
   - ロックアウトは `%APPDATA%\FocusVisionPCVR\lockout.json` に残るので、SteamVR やコンパニオンを再起動しても解けない
   - **対処:** 時間が過ぎるのを待ち、新しい PIN を入力する

### TLS 証明書の警告 / 検証エラー

**原因候補と対処:**

1. **初回接続時の TOFU (Trust On First Use) ピン留め**
   - HMD は最初にペアリングできた PC の自己署名証明書 (の SHA-256) を覚え、以後それと違う証明書の PC にはつながない
   - PC の証明書は `%APPDATA%\FocusVisionPCVR\tls_identity.bin` に保存され、SteamVR の再起動やドライバの再インストールでは変わらない。変わるのは、このファイルを消した (読めなくなった) ときと、別の PC につなぐとき
   - **対処:** HMD 側でアプリのデータを消去 (Settings → Apps → Focus Vision PCVR → Clear Data) してから、ペアリングし直す

---

## 映像関連

### 映像が出ない / 真っ黒のまま

**原因候補と対処:**

1. **SteamVR がドライバを認識していない**
   - コンパニオンアプリ Home タブの「SteamVR Driver」が `Installed` か確認。SteamVR 側では、設定の「Startup / Shutdown」→「Manage Add-ons」に Focus Vision PCVR が出ていて有効になっているか
   - **対処:** `Not installed` なら Home タブの「Install Driver」(インストーラと同じく、アプリと一緒に入ったドライバを SteamVR に登録する)、またはインストーラをもう一度実行する → SteamVR を再起動。下の「SteamVR 関連」も参照

2. **NVENC エンコーダ初期化失敗**
   - NVIDIA ドライバが古い / 非対応 GPU。このときドライバは映像を流さない
   - **対処:** SteamVR のログ `vrserver.txt` (Export Logs の zip の `steamvr/`) で `NVENC:` で始まる行 (`nvEncInitializeEncoder failed` など) を確認。NVIDIA ドライバを 551.76 以降 (NVENC API 12.2) に更新

3. **HMD 側のデコーダエラー**
   - HMD ログ (`adb logcat -s FocusVision`) に `MediaCodec` を含むエラー行がないか確認
   - **対処:** HMD を再起動。それでも続く場合は codec を H.264 に切替 (コンパニオンの Settings タブ → Video Codec。SteamVR の再起動で反映)

### 映像がカクつく / フレーム落ちが多い

**原因候補と対処:**

1. **Wi-Fi 信号が弱い**
   - HMD 内の接続品質の表示 (信号バー。パケットロスで緑 → 黄 → 赤) が黄〜赤になっていないか
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
   - 帯域不足を検知して自動的にビットレートを下げている (`bitrate_mbps` が上限)
   - **対処:** Home タブの Streaming 欄の Mbps (いまのビットレート) と、セッションログ (`sessions\session_*.jsonl`) の `bitrate_mbps` の推移を見る。安定して 30 Mbps 未満なら Wi-Fi 環境見直しが本質的解決

4. **GPU が熱で性能低下している (4時間以上連続稼働など)**
   - PC の通気を確認する
   - thermal governor (`[thermal]`、温度でビットレートの上限を下げる) は NVML に対応したビルド (`nvml` feature) でだけ動く。配布版は対応していないので、`enabled = true` にしても engine.log に警告が出るだけ

### 映像にノイズ / 緑のブロック / IDR ストーム

**原因候補と対処:**

1. **パケットロスが多くて FEC で復旧しきれていない**
   - Home タブの Subsystems の `Loss` が 5% 超 (赤)
   - **対処:** Wi-Fi 環境改善 (上記)。FEC の冗長度は適応制御で `[network] fec_redundancy_min`〜`fec_redundancy_max` (既定 0.05〜0.40) の間を動くので、上限の `fec_redundancy_max` を上げる (最大 1.0。そのぶん帯域を使う)

2. **HMD がキーフレーム (IDR) を頻繁に要求している**
   - HMD はフレームを組み立てられないと IDR を要求する。engine.log に `IDR_REQUEST suppressed (rate limit, … total)` が続く (エンジンは 1 秒に 2 回までしか応じない)
   - **対処:** `[network] slice_count` を下げる (例: 4 → 2)、または `slice_fec_enabled = false` でフレーム単位の FEC に戻す

### 映像はあるが視野中央だけ高画質、周辺はモザイク

`[foveated] enabled = true` にしている場合は**仕様**です — Foveated Encoding (既定では無効) が効いています。

- 視線から離れた部分の画質を意図的に下げて帯域を節約する。削減量は実機でまだ計測していない
- HMD から視線が届いている間だけ効く (一度も届かなければ全体を同じ画質で送る。途中で届かなくなると、最後の視線の位置のまま)
- 違和感がある場合は `config/local.toml` で `[foveated] enabled = false` に戻すか、`preset = "subtle"` にする (コンパニオンの Settings タブには項目がない)

---

## 音声関連

### 音声が聞こえない

**原因候補と対処:**

1. **PC 側に音声出力デバイスがない / WASAPI loopback 失敗**
   - 例: ヘッドホンを抜いた瞬間に Windows が「再生デバイスなし」状態になる
   - **対処:** PC に音声出力 (内蔵スピーカー、有線/Bluetooth ヘッドホン、HDMI モニター等) を1つ接続する。エンジンは 2 秒ごとに Windows の既定の出力デバイスを確かめ、配信中でもそれに切り替える (engine.log に `Audio output device: '…' → '…'`)。開けなかったデバイスは 30 秒後にもう一度試す

2. **HMD 側 AAudio 初期化失敗**
   - **対処:** HMD を再起動

3. **Discord などの仮想オーディオドライバが loopback をブロック**
   - **対処:** PC のサウンド設定で既定の出力デバイスを物理デバイスに戻す

### 音声と映像がずれている

**原因候補と対処:**

1. **PC ↔ HMD のクロック同期ずれ (RTP timestamp の累積誤差)**
   - 数十分の長時間プレイで発生しやすい
   - **対処:** いったん切断して再接続するとリセットされる

（音声のフレームは常に 10 ms で、設定では変わらない。以前ここで案内していた `[audio] frame_size_ms` は効かない設定だった）

---

## ハプティクス / トラッキング

### コントローラーが SteamVR に表示されない

**原因候補と対処:**

1. **HMD 側でコントローラーがペアリングされていない**
   - HMD のシステム設定でコントローラー接続状態を確認
   - **対処:** コントローラーを再ペアリング

2. **HMD のアプリがコントローラーの入力を読めていない**
   - コントローラーは HMD から入力が届いてから SteamVR に現れる。HMD ログ (`adb logcat -s FocusVision`) の `OpenXR extensions: Focus 3 controller yes/no` を確認
   - **対処:** HMD のシステムを更新する / コントローラーを再ペアリングする

3. **SteamVR 側の Oculus Touch の定義が見つからない**
   - ドライバはコントローラーを Oculus Touch (Quest 2) として見せ、ボタン割り当てと描画モデルは SteamVR に同梱の Oculus ドライバのもの (`{oculus}/input/touch_profile.json`) を使う
   - **対処:** SteamVR を最新版に更新する (Oculus ドライバのファイルは SteamVR と一緒に入る)

### コントローラーの振動が来ない

**原因候補と対処:**

1. **HAPTIC_EVENT (0x38) パイプラインが詰まっている**
   - `engine.log` で `Haptic event dropped (total: …)` がないか確認 (100 件ごとに 1 行)
   - **対処:** 多くの場合は一時的。続く場合はバグ報告を

### 顔表情が VRChat に反映されない (Face Tracking)

**原因候補と対処:**

1. **HMD の顔トラッキングが使えていない**
   - HMD ログ (`adb logcat -s FocusVision`) で `OpenXR extensions: … facial tracking yes/no` と `FacialTracker:` の行を確認 (`XR_HTC_facial_tracking not available` なら、ランタイムが顔トラッキングを提供していない)
   - **対処:** HMD のシステムを更新する。顔トラッカーを付けている場合は装着を確認

2. **PC 側で無効になっている**
   - コンパニオンの Settings タブの「Face tracking → VRChat OSC」(`[face_tracking] enabled`) がオンか確認。SteamVR の再起動で反映

3. **VRChat OSC ポートが違う**
   - 既定 9000。VRChat 側で別ポートを設定している場合は `config/local.toml` の `[face_tracking] osc_port` を合わせる

4. **アバターに対応ブレンドシェイプがない**
   - **対処:** VRChat 用 FT 対応アバターを選択

---

## SteamVR 関連

### SteamVR が「ドライバが見つかりません」と表示

**原因候補と対処:**

1. **ドライバが SteamVR に登録されていない**
   - インストーラは SteamVR (どの Steam ライブラリにあっても) の vrpathreg でドライバを登録する。インストール時に SteamVR が入っていなかった、または SteamVR の実行中だった場合は登録されない
   - **対処:** SteamVR をインストールして一度起動してから、コンパニオンの Home タブの「Install Driver」を押す (またはインストーラをもう一度実行する)。SteamVR を再起動すると読み込まれる

2. **SteamVR の別バージョン (Beta など) を使っている**
   - **対処:** Steam クライアントで SteamVR を安定版に切り替える

### SteamVR がクラッシュする / ハングする

**原因候補と対処:**

1. **Direct Mode 競合 (他の VR ドライバが残っている)**
   - **対処:** SteamVR の設定の「Startup / Shutdown」→「Manage Add-ons」で、不要な他社製ドライバを無効化

---

## ログを集める

上記で解決しない場合、サポート/Issue 用にログを収集します:

1. PC 側コンパニオンアプリ → Settings タブ → Diagnostics の **Export Logs (zip)** ボタン
2. ダウンロードフォルダに ZIP ができる。中身:
   - エンジンのログ (`%APPDATA%\FocusVisionPCVR\engine.log` と前回分の `engine.prev.log`)、status.json、コンパニオンの設定 (`config\local.toml`)
   - 直近 5 セッションのセッションログ (`%APPDATA%\FocusVisionPCVR\sessions\session_*.jsonl`。配信中 10 秒ごとに PC 側の遅延と送信 fps・ビットレート・ロス率・FEC・HMD の fps とデコード時間・メモリを 1 行ずつ記録。7 日で消える)
   - SteamVR のログ (vrserver.txt、vrcompositor.txt と、それぞれ前回分。ドライバのログは vrserver.txt に出る)
   - HMD 側 logcat (ADB で接続しているとき)
   - システム情報 (Windows のバージョン、GPU とドライババージョン)
3. PII (IP アドレス、MAC アドレス、SSID、メールアドレス、ユーザー名を含むパス、PIN) は自動でマスクされる
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

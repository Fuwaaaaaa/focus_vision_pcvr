# 実機テスト手順書

Focus Vision PCVR の実機テスト手順。PC + VIVE Focus Vision + Wi-Fi 環境が必要。

---

## 1. 必要な機材

| 機材 | 要件 |
|------|------|
| Windows PC | NVIDIA GPU (NVENC対応)、Wi-Fi 5/6 |
| VIVE Focus Vision | 開発者モード ON、同一Wi-Fiネットワーク |
| USB-C ケーブル | APKデプロイ用（初回のみ） |
| SteamVR | インストール済み |

## 2. ネットワーク要件

```
ファイアウォールで以下を許可:

  TCP 9944  (制御チャンネル — TLS)     PC ↔ HMD
  UDP 9946  (ビデオストリーム)          PC → HMD
  UDP 9947  (トラッキングデータ)        HMD → PC
  UDP 9948  (オーディオストリーム)       PC → HMD
```

Wi-Fiは **5GHz帯** 必須（2.4GHzではビットレート不足）。

---

## 3. セットアップ

### 3A. PC側

```bash
# リリースからインストール（推奨）
# GitHub Releases から FocusVision-Companion-v1.1.0.zip をダウンロード
# 展開して focus-vision.exe を起動

# ソースからビルドする場合:
./build.bat
cargo run -p focus-vision-companion
```

**コンパニオンアプリで:**
1. 「Install Driver」をクリック → SteamVRドライバーを登録
2. SteamVR を起動
3. **6桁のPIN** が表示されるのを確認

### 3B. HMD側（初回のみ）

```bash
# Focus VisionをUSBでPCに接続
# コンパニオンアプリの「Deploy」タブで:
# 1. APKファイルを選択（FocusVision-Client-v1.1.0.apk）
# 2. デバイスが表示されることを確認
# 3. 「Install APK on All Devices」をクリック
```

### 3C. 接続

1. Focus VisionとPCを **同一Wi-Fiネットワーク** に接続
2. HMDでFocus Vision PCVRアプリを起動
3. PC画面に表示された **6桁PIN** をHMDに入力
4. 「Connected」が表示されたらストリーミング開始

---

## 4. 基本動作テスト

### 4A. 接続テスト

| テスト項目 | 確認方法 | 期待値 |
|-----------|---------|--------|
| TCP接続 | コンパニオンアプリで「Connected」表示 | 緑インジケーター |
| PIN表示 | コンパニオンアプリのHome画面 | 6桁の数字 |
| PIN入力 | HMDで正しいPINを入力 | 接続成功 |
| PIN間違い | 5回間違えたPINを入力 | 300秒ロックアウト |
| 自動再接続 | Wi-Fiを一時切断→復帰 | 1-16秒で再接続 |

### 4B. ビデオストリーミング

| テスト項目 | 確認方法 | 期待値 |
|-----------|---------|--------|
| FPS | コンパニオンアプリの数値 | 90fps |
| レイテンシー | コンパニオンアプリの数値 | ≤50ms |
| ビットレート | コンパニオンアプリの数値 | ~80 Mbps |
| 映像品質 | HMDで目視 | SteamVRの映像が表示 |
| タイムワープ | 頭を素早く回す | カクつき最小 |

### 4C. オーディオ

| テスト項目 | 確認方法 | 期待値 |
|-----------|---------|--------|
| PC音声がHMDで聞こえる | SteamVRで音楽再生 | HMDスピーカーから出力 |
| 遅延 | 映像と音声の同期 | <100ms |
| 音質 | 主観評価 | クリア、ノイズなし |

### 4D. トラッキング＋コントローラー

| テスト項目 | 確認方法 | 期待値 |
|-----------|---------|--------|
| 6DoF頭部追跡 | HMDを動かす | SteamVR内で追従 |
| コントローラー | ボタン/トリガー操作 | SteamVR入力に反映 |
| 視線追跡 | HMDの視線情報送信 | logcatでgaze_valid=1 |

### 4E. HMDオーバーレイ

| テスト項目 | 確認方法 | 期待値 |
|-----------|---------|--------|
| 信号バー表示 | VR体験中に左下を確認 | 3本のバーアイコン |
| 色変化 | Wi-Fi帯域を絞る | 緑→黄→赤 |

---

## 5. H.265 vs H.264 レイテンシー計測

### 手順

```bash
# 1. H.265テスト（デフォルト）
#    コンパニオンアプリでCodec: H.265を選択（Settings→Video Codec）
#    SteamVRでゲームを起動、5分以上ストリーミング

# 2. HMD側のデコードレイテンシーを記録
adb logcat | grep "decode latency"
# 出力例: VideoDecoder: decode latency avg=15000us (90 frames)

# 3. H.264に切替
#    コンパニオンアプリでCodec: H.264を選択
#    エンジンを再起動（アプリを再起動）

# 4. 同じゲームで5分以上ストリーミング
adb logcat | grep "decode latency"

# 5. 結果を比較
```

### 記録テンプレート

```
日付: ____
ゲーム: ____
Wi-Fi帯域: 5GHz / 6GHz

H.265:
  平均デコードレイテンシー: ____us
  サンプル数: ____
  主観画質: ____/10

H.264:
  平均デコードレイテンシー: ____us
  サンプル数: ____
  主観画質: ____/10

結論: H.265 / H.264 を採用（理由: ____）
```

---

## 6. Foveated Encoding テスト

### 前提条件
- NVENC SDK構造体オフセットの検証が必要（`nvenc_encoder.h`のインライン定義）
- **初回は必ず短時間テスト**（クラッシュリスクあり）

### 手順

```bash
# 1. config/local.toml で foveated を有効化
[foveated]
enabled = true

# 2. エンジンを再起動
# 3. HMDで視線を動かしながらストリーミング
# 4. 確認:

# PC側: foveated encodingが有効か
# SteamVRログで "Foveated encoding enabled" を検索

# HMD側: 視線データが送信されているか
adb logcat | grep "gaze"

# 視覚的確認: 周辺部のぼやけが見えるか（QP+15の効果）
```

### チェック項目

| テスト | 確認方法 | 期待値 |
|--------|---------|--------|
| 視線追跡有効 | logcatでgaze_valid | 1 |
| QP delta map適用 | NVENC出力ログ | foveated=true |
| 帯域削減 | ビットレート比較 | 10-30%削減 |
| 画質（中心部） | 目視 | フル品質 |
| 画質（周辺部） | 目視 | 軽いぼやけ |
| クラッシュなし | 5分連続動作 | 安定 |

---

## 7. TLS E2Eテスト

```bash
# PC側: TLSが有効か確認
# エンジンログで "TLS enabled. Cert fingerprint: ..." を検索

# 外部ツールで確認:
openssl s_client -connect localhost:9944

# 期待値: TLS 1.3ハンドシェイク成功、自己署名証明書表示

# HMD側: TLS接続ログ
adb logcat | grep "TLS"
# 期待値: "TLS handshake complete (cipher: ...)"
```

---

## 8. 安定性テスト

| テスト | 手順 | 合格基準 |
|--------|------|---------|
| 10分連続 | SteamVRゲームを10分プレイ | FPS低下なし、クラッシュなし |
| 30分連続 | 30分プレイ | メモリ使用量安定 |
| Wi-Fi切断→復帰 | ルーターのWi-Fiを5秒OFF | 16秒以内に再接続 |
| HMDスリープ→復帰 | HMDを外して30秒→再装着 | 自動再接続 |
| SteamVR再起動 | SteamVRを終了→再起動 | 再接続可能 |

---

## 9. トラブルシューティング

### 接続できない

```bash
# 1. 同一ネットワークか確認
ipconfig                           # PC側IP
adb shell ip addr show wlan0       # HMD側IP

# 2. ポートが開いているか
netstat -an | grep 9944            # TCP listening?

# 3. ファイアウォール
# Windows Defender → 受信の規則 → focus-vision.exe を許可

# 4. SteamVRドライバー
# コンパニオンアプリで "Installed" が表示されているか
```

### 映像が表示されない

```bash
# 1. UDPパケットが届いているか
adb logcat | grep "packet"

# 2. デコーダーエラー
adb logcat | grep "MediaCodec"

# 3. NAL検証エラー（FEC復元失敗）
adb logcat | grep "NAL validation"
# → IDRリクエストが自動送信されるはず
```

### レイテンシーが高い

```bash
# 1. 各段階のレイテンシー確認
adb logcat | grep "decode latency"   # デコード
# status.json の latency_us            # E2E

# 2. Wi-Fi品質
adb shell dumpsys connectivity | grep "Wi-Fi"

# 3. 5GHz帯を使っているか確認
# 2.4GHzでは帯域不足でFPSが低下する
```

### ログエクスポート

コンパニオンアプリの Settings → Diagnostics → 「Export Logs (zip)」
- PC側ログ + HMD logcat + システム情報を自動収集
- `~/Downloads/focus-vision-logs-{timestamp}.zip` に保存

---

## 10. テスト結果サマリーテンプレート

```
=== Focus Vision PCVR 実機テスト結果 ===
日付: ____
テスター: ____
PC: ____ (GPU: ____)
HMD: VIVE Focus Vision
Wi-Fi: ____ (5GHz/6GHz)
ビルド: v1.1.0 (commit: ____)

基本接続:     PASS / FAIL
ビデオ:       PASS / FAIL  (FPS: ____, Latency: ____ms)
オーディオ:   PASS / FAIL
トラッキング: PASS / FAIL
コントローラー: PASS / FAIL
HMDオーバーレイ: PASS / FAIL
TLS:          PASS / FAIL
10分安定性:   PASS / FAIL

H.265 decode: ____us avg
H.264 decode: ____us avg
推奨codec:    H.265 / H.264

Foveated:     PASS / FAIL / SKIP
帯域削減:     ____%

備考:
____
```

---

## 11. v1.2/v1.3 新機能テスト

### 11A. Face Tracking (EMAスムージング)

1. `config/local.toml` で `[face_tracking] enabled = true, smoothing = 0.6` を設定
2. VRChat でアバターを表示
3. 顔を動かしてblendshapeが反映されることを確認
4. smoothing を 0.0（生データ）と 0.9（最大ラグ）で比較

| チェック項目 | 期待値 |
|-------------|--------|
| lip blendshapes 反映 | 口の動きがアバターに表示 |
| eye blendshapes 反映 | まばたき・視線がアバターに表示 |
| smoothing=0.0 | ジッター多いが即応答 |
| smoothing=0.9 | 滑らかだが遅延あり |

### 11B. ハプティクスフィードバック

1. SteamVR ゲームで武器を撃つ/物を掴む
2. コントローラーの振動を確認

| チェック項目 | 期待値 |
|-------------|--------|
| 左コントローラー振動 | SteamVR イベントに応じて振動 |
| 右コントローラー振動 | 同上 |
| 振動なしゲーム | エラーなし |

### 11C. 睡眠モード

1. `config/local.toml` で `[sleep_mode] timeout_seconds = 30` に設定（テスト用短縮）
2. HMD を静置して30秒待つ

| チェック項目 | 期待値 |
|-------------|--------|
| 30秒後にビットレート低下 | 80→8 Mbps |
| 画面暗転 | ダイミングオーバーレイ表示 |
| 頭を動かすと復帰 | 即座にビットレート復帰 |
| ボタン押下で復帰 | 同上 |

### 11D. HMDダッシュボード

1. メニューボタンを押してダッシュボード表示
2. ビットレート/codec表示を確認

| チェック項目 | 期待値 |
|-------------|--------|
| メニューボタンでトグル | パネル表示/非表示 |
| ビットレート表示 | 現在値が正しい |
| codec表示 | H.265/H.264 が正しい |

### 11E. コンフィグバリデーション

1. `config/default.toml` に不正値を設定: `bitrate_mbps = 0`
2. エンジンを起動

| チェック項目 | 期待値 |
|-------------|--------|
| bitrate_mbps=0 | ログに警告、デフォルト80に復帰 |
| tcp_port=80 | ログに警告、デフォルト9944に復帰 |
| smoothing=NaN | ログに警告、デフォルト0.6に復帰 |

### 11F. コントローラー タッチ＋デッドゾーン

1. SteamVR でコントローラー入力テスト
2. トリガー/グリップに触れる（押さない）

| チェック項目 | 期待値 |
|-------------|--------|
| trigger touch 検知 | SteamVR に touch 状態送信 |
| grip touch 検知 | 同上 |
| thumbstick touch 検知 | 同上 |
| スティック微小入力 | デッドゾーン内はゼロ |
| バッテリーレベル | 実際の値（100%固定でない） |

# E2E テスト手順書 — Focus Vision PCVR

帰宅後にFocus Vision実機でテストするための手順。

## 前提条件

- Windows PC + NVIDIA GPU (GTX 1060以上)
- VIVE Focus Vision (開発者モード有効化済み)
- USB-Cケーブル
- SteamVR インストール済み
- Wi-Fi 5GHz 同一ネットワーク

## Step 1: ビルド

```bash
# Rust streaming engine + companion app
cargo build --release -p streaming-engine
cargo build --release -p focus-vision-companion

# C++ driver (CMake)
cd driver
mkdir -p build && cd build
cmake .. -G "Visual Studio 17 2022" -A x64
cmake --build . --config Release
cd ../..

# Android client (要 Android NDK)
cd client
./gradlew assembleDebug
cd ..
```

## Step 2: ドライバーインストール

### 方法A: コンパニオンアプリ経由
```bash
./target/release/focus-vision.exe
```
HomeタブのInstall Driverボタンを押す。

### 方法B: 手動
```bash
# SteamVRのドライバーフォルダーにコピー
cp -r driver/build/Release/focus_vision_pcvr/ \
  "C:/Program Files (x86)/Steam/steamapps/common/SteamVR/drivers/"
```

## Step 3: APKデプロイ

### 方法A: コンパニオンアプリ経由
1. Focus VisionをUSBで接続
2. Deployタブ → デバイスが表示される
3. APKファイルを選択 → Install

### 方法B: ADB手動
```bash
adb install -r client/app/build/outputs/apk/debug/app-debug.apk
```

## Step 4: テスト実行

### 4.1 接続テスト
1. SteamVRを起動
2. コンパニオンアプリでPINを確認
3. Focus VisionでアプリをHMDから起動
4. PINを入力
5. **期待結果**: 接続成功、SteamVR上にHMDが表示される

### 4.2 ビデオストリーミングテスト
1. SteamVR Homeが表示されるか確認
2. 頭を動かしてトラッキングが追従するか
3. **確認項目**:
   - [ ] 映像が表示される
   - [ ] フレームレートが安定 (90fps目標)
   - [ ] レイテンシーが体感50ms以下
   - [ ] パケットロス時にIDR要求→回復する

### 4.3 コントローラーテスト
1. コントローラーのボタン入力
2. トリガー、グリップ、サムスティック
3. **確認項目**:
   - [ ] 6DoF追従
   - [ ] ボタン入力がSteamVRに反映

### 4.4 オーディオテスト
1. SteamVR Homeの環境音が聞こえるか
2. ゲームを起動して音が出るか
3. **確認項目**:
   - [ ] PC側: WASAPI loopback captureが動作
   - [ ] HMD側: Opusデコード → スピーカー出力 (要NDK実装)
   - [ ] リップシンク (映像と音のズレ)

### 4.5 ストレステスト
1. 5分間連続ストリーミング
2. Wi-Fiを一時的に遮断 → 再接続
3. **確認項目**:
   - [ ] 接続切断 → 自動再接続
   - [ ] メモリリークなし (タスクマネージャーで確認)
   - [ ] CPU/GPU使用率が安定

## Step 5: H.265 vs H.264 テスト

TODOS.mdの調査項目。

```toml
# config/local.toml (H.264に切り替え)
[video]
codec = "h264"
```

1. H.265で5分ストリーミング → レイテンシー記録
2. config変更 → H.264で5分ストリーミング → レイテンシー記録
3. 画質差を主観評価

## トラブルシューティング

### SteamVRがドライバーを認識しない
```bash
# ドライバーログ確認
cat "%LOCALAPPDATA%/openvr/logs/vrserver.txt" | grep "Focus Vision"
```

### 接続できない
- PCとHMDが同じネットワーク上か確認
- ファイアウォールでUDP 9944-9948を許可
- `netstat -an | grep 9944` でポートが使用中でないか

### 映像が表示されない
- NVENCがテストパターンモードになっていないか確認
  (コンパニオンアプリのログで "Real NVENC initialized" を確認)
- `nvEncodeAPI64.dll` がシステムPATHに存在するか

### 音が出ない
- コンパニオンアプリのSettingsでAudioがEnabledか確認
- PCのデフォルト出力デバイスが正しいか
- HMD側のOpusデコード実装がまだスタブの場合は音は出ない (TODOS.md参照)

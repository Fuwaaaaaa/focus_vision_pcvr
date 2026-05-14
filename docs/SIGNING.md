# Code Signing Guide (Windows Authenticode + Android APK)

CI には署名パイプラインが組み込まれていますが、**証明書/キーストアは secrets として明示的に設定するまで自動的に未署名ビルドになります**。本ドキュメントは「証明書を入手した後どこに置けば本物署名に切り替わるか」を説明します。

---

## Windows Authenticode (focus-vision.exe + Setup.exe)

### 概要

CI ワークフロー (`.github/workflows/build.yml`) の `companion-build` と `installer-build` ジョブは、以下の secrets が設定されているときだけ `scripts/sign-windows.ps1` を実行します:

| Secret 名 | 内容 |
|----------|------|
| `WINDOWS_PFX_BASE64` | `.pfx` 証明書ファイルの base64 エンコード |
| `WINDOWS_PFX_PASSWORD` | `.pfx` のパスワード |

両方が設定されると:
1. `focus-vision.exe` (コンパニオンアプリ) が署名される
2. `FocusVision-<version>-Setup.exe` (NSIS インストーラ) も署名される
3. CI ログに `Signed: ... ` と表示される

どちらか欠けると、その step は `if: ${{ secrets.WINDOWS_PFX_BASE64 != '' }}` でスキップされ、`Smoke-verify the installer` の出力に `NotSigned` と記録されます。

### 証明書の選択肢

| 種別 | 価格目安 | SmartScreen 即時通過 | 用途 |
|------|---------|-------------------|------|
| **EV Code Signing Certificate** | $200-800/年 | ✅ 即時 | プロダクション配布、ダウンロード数が少ない初期に有効 |
| **Standard Code Signing Certificate** | $100-300/年 | ❌ ダウンロード数の蓄積が必要 | コスト重視、ある程度ユーザーがいるなら時間で解決 |
| **自己署名 (テスト用)** | 無料 | ❌ 永遠に警告 | CI パイプライン検証のみ |
| **未署名** | 無料 | ❌ 永遠に警告 | デフォルト (現状) |

EV 証明書の発行元: DigiCert, Sectigo, GlobalSign, SSL.com など。プロセスは:
1. 個人または法人実在性審査 (1-2週間)
2. ハードウェアトークン (USB) で配送される ← EV は秘密鍵をトークンから取り出せない設計
3. トークン経由でしか signing できないため、CI で直接使うには Azure Key Vault や Cloud Signing Service 経由が必要

**Standard 証明書なら .pfx エクスポート可能** なので、CI で直接使える。

### secrets 設定手順 (Standard 証明書の場合)

GitHub UI:
1. リポジトリの **Settings → Secrets and variables → Actions → New repository secret**
2. Name: `WINDOWS_PFX_BASE64`、Value: 以下のコマンドで生成
   ```powershell
   [Convert]::ToBase64String([IO.File]::ReadAllBytes("path\to\cert.pfx"))
   ```
   出力された長い文字列をペースト
3. Name: `WINDOWS_PFX_PASSWORD`、Value: .pfx 作成時に設定したパスワード
4. 次の CI ビルドから自動的に署名が有効化される

CLI (`gh` ツール):
```sh
gh secret set WINDOWS_PFX_BASE64 -b "$(base64 -w0 cert.pfx)"
gh secret set WINDOWS_PFX_PASSWORD -b 'your-pfx-password'
```

### ローカル動作確認

```powershell
$env:WINDOWS_PFX_BASE64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx"))
$env:WINDOWS_PFX_PASSWORD = "your-password"

# ビルド
cargo build --release -p focus-vision-companion
# 署名 (本体)
pwsh ./scripts/sign-windows.ps1 -FilePath target/release/focus-vision.exe
# インストーラ生成
makensis installer/focus_vision.nsi
# 署名 (インストーラ)
$exe = Get-ChildItem out/FocusVision-*-Setup.exe | Select-Object -First 1
pwsh ./scripts/sign-windows.ps1 -FilePath $exe.FullName

# 検証
Get-AuthenticodeSignature $exe.FullName
```

`Status: Valid` が出れば成功。

### タイムスタンプサーバ

`sign-windows.ps1` のデフォルトは `http://timestamp.digicert.com`。証明書の有効期限が切れた後も署名が信頼され続けるために必須です。

DigiCert が落ちている場合は `-TimestampUrl http://timestamp.sectigo.com` などに差し替え可能。

---

## Android APK 署名 (Phase D3 で追加予定)

現状の APK は `gradle assembleDebug` で生成しており、Android のデバッグキーストアで自動署名されます。**サイドロード配布には問題なし、Play Store には不可** です。

Phase D3 で `assembleRelease` + 永続キーストアを導入する予定。設定先 secrets:

| Secret 名 (予定) | 内容 |
|-----------------|------|
| `ANDROID_KEYSTORE_BASE64` | release.keystore の base64 |
| `ANDROID_KEYSTORE_PASSWORD` | キーストアのパスワード |
| `ANDROID_KEY_ALIAS` | キーのエイリアス |
| `ANDROID_KEY_PASSWORD` | キーのパスワード |

これも secrets 未設定なら ephemeral keystore でフォールバック予定。

---

## SteamVR ドライバ DLL の署名 (今後検討)

`driver_focus_vision_pcvr.dll` も Authenticode 署名できれば理想ですが、現状の Phase D2 では未対応です。理由:

- SteamVR ドライバはユーザーモードで動くため、署名なしでもロード自体は可能
- ただし、署名されていると Defender や EDR ツールでの誤検知が減る
- 将来 Phase D2.1 として `installer-build` の前に DLL を署名する step を追加する余地あり (`sign-windows.ps1` がそのまま使える)

---

## 監査 / トラブル対処

### 「ビルドは緑だけど署名されていない」

原因: secrets が設定されていないので step が `if` でスキップ。CI ログの `Sign focus-vision.exe (Authenticode)` step が「Skipped」になっていることを確認。secrets 設定 (上記手順) で解決。

### 「signtool not found」

原因: Windows SDK が CI runner に入っていない。windows-latest にはデフォルトで入っているはず。`sign-windows.ps1` が `C:\Program Files (x86)\Windows Kits\10\bin\<ver>\x64\signtool.exe` を再帰検索するので、SDK が一切ない runner では失敗します。その場合 step に `windows-sdk` を明示インストールする必要があります。

### 「SignTool Error: A required certificate is not within its validity period」

証明書の有効期限切れ。新しい証明書を取得し直して GitHub secrets を更新。

### 「The specified PFX password is not correct」

`WINDOWS_PFX_PASSWORD` の値が間違っている。secrets を更新。

### CI で署名後に再度 SmartScreen が警告する

Standard 証明書の場合、ダウンロード数がある程度 (数百〜数千) 累積するまで Microsoft の reputation システムに認識されません。EV 証明書なら最初から OK。

---

## 関連ファイル

- `.github/workflows/build.yml` — 署名 step (`companion-build` + `installer-build` 内)
- `scripts/sign-windows.ps1` — 共通署名スクリプト
- `installer/focus_vision.nsi` — NSIS インストーラ (生成された .exe が署名対象)

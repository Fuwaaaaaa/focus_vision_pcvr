; ============================================================================
; Focus Vision PCVR — NSIS Installer
; ============================================================================
;
; Builds a single FocusVision-<version>-Setup.exe that:
;   - Installs the companion app, fonts, default config, and OpenVR driver
;     tree under "$PROGRAMFILES64\Focus Vision PCVR\"
;   - Auto-registers the SteamVR driver via vrpathreg (path resolved from
;     the registry — works regardless of which drive Steam is on)
;   - Creates Start Menu and Desktop shortcuts
;   - Writes an uninstaller that reverses everything (incl. vrpathreg
;     removedriver) but leaves the user's %APPDATA% config and recordings
;     intact
;
; Build locally with NSIS 3.x:
;   makensis installer/focus_vision.nsi
;
; CI builds via .github/workflows/build.yml (windows-latest already ships NSIS).
;
; ============================================================================

Unicode true
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"
!include "x64.nsh"

; ---- Constants -------------------------------------------------------------

!define APP_NAME       "Focus Vision PCVR"
!define APP_VERSION    "3.0.0-rc3"
; NSIS VIProductVersion requires 4-component numeric (MAJOR.MINOR.PATCH.BUILD).
; Pre-release tag is encoded by reserving the 4th component for the rc number:
;   3.0.0-rc3  -> 3.0.0.3
;   3.0.0      -> 3.0.0.1000 (final, sorts above any rc)
!define APP_VERSION_NUMERIC "3.0.0.3"
!define APP_PUBLISHER  "Fuwaaaaaa"
!define APP_URL        "https://github.com/Fuwaaaaaa/focus_vision_pcvr"
!define APP_KEY        "FocusVisionPCVR"
!define APP_EXE        "focus-vision.exe"
!define DRIVER_DIRNAME "focus_vision_pcvr"

; Reg keys
!define REG_APP        "Software\${APP_KEY}"
!define REG_UNINST     "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_KEY}"

; ---- Installer metadata ----------------------------------------------------

Name "${APP_NAME}"
OutFile "..\out\FocusVision-${APP_VERSION}-Setup.exe"
InstallDir "$PROGRAMFILES64\${APP_NAME}"
InstallDirRegKey HKLM "${REG_APP}" "InstallDir"
RequestExecutionLevel admin    ; vrpathreg + ProgramFiles writes need admin

; Embedded version info so Windows shows clean metadata in Properties
VIProductVersion "${APP_VERSION_NUMERIC}"
VIAddVersionKey "ProductName"     "${APP_NAME}"
VIAddVersionKey "FileDescription" "${APP_NAME} Installer"
VIAddVersionKey "FileVersion"     "${APP_VERSION}"
VIAddVersionKey "ProductVersion"  "${APP_VERSION}"
VIAddVersionKey "LegalCopyright"  "© ${APP_PUBLISHER}"
VIAddVersionKey "CompanyName"     "${APP_PUBLISHER}"

; ---- MUI2 pages ------------------------------------------------------------

!define MUI_ABORTWARNING
!define MUI_ICON   "${NSISDIR}\Contrib\Graphics\Icons\modern-install.ico"
!define MUI_UNICON "${NSISDIR}\Contrib\Graphics\Icons\modern-uninstall.ico"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES

; Offer to launch the app after install completes
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Focus Vision PCVR を起動する"
!define MUI_FINISHPAGE_LINK "${APP_URL}"
!define MUI_FINISHPAGE_LINK_LOCATION "${APP_URL}"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

; Japanese first (project is Japanese-first); English as secondary
!insertmacro MUI_LANGUAGE "Japanese"
!insertmacro MUI_LANGUAGE "English"

; ---- Function: VC++ runtime prerequisite check ----------------------------
;
; The Rust streaming engine (staticlib linked into the SteamVR driver DLL)
; and the companion app are built with the MSVC toolchain and depend on the
; Visual C++ 2015-2022 Redistributable (x64). Without it the driver fails to
; load with cryptic "missing MSVCP140.dll" errors that show up only inside
; SteamVR's log — almost impossible for end users to diagnose. Aborting in
; .onInit shows a clear message instead of letting them complete a broken
; install.
Function .onInit
    Push $0
    Push $1

    ; vc_redist.x64 writes Installed=1 (DWORD) on success.
    SetRegView 64
    ReadRegDWORD $0 HKLM "SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64" "Installed"
    SetRegView default

    ${If} $0 == "1"
        Pop $1
        Pop $0
        Return
    ${EndIf}

    MessageBox MB_ICONEXCLAMATION|MB_OK \
        "Microsoft Visual C++ 2015-2022 Redistributable (x64) がインストールされていません。$\n$\n\
このランタイムは Focus Vision PCVR ドライバの動作に必須です。$\n$\n\
以下から vc_redist.x64.exe をダウンロード・インストールしてから、$\n\
再度このセットアップを実行してください:$\n$\n\
https://aka.ms/vs/17/release/vc_redist.x64.exe"

    ; Open the download URL so the user doesn't have to retype it.
    ExecShell "open" "https://aka.ms/vs/17/release/vc_redist.x64.exe"

    Pop $1
    Pop $0
    Abort
FunctionEnd

; ---- Section: main install -------------------------------------------------

Section "Focus Vision PCVR (必須)" SecMain
    SectionIn RO    ; required — cannot deselect

    ; Wipe a prior install at the same target dir so leftover bits from a
    ; previous version (e.g. stale driver DLL with different ABI) don't
    ; ride along into the new install.
    DetailPrint "既存ファイルを整理しています..."
    Delete "$INSTDIR\${APP_EXE}"
    RMDir /r "$INSTDIR\fonts"
    RMDir /r "$INSTDIR\config"
    RMDir /r "$INSTDIR\driver"

    SetOutPath "$INSTDIR"
    DetailPrint "コンパニオンアプリを展開しています..."
    File "..\target\release\${APP_EXE}"

    SetOutPath "$INSTDIR\fonts"
    DetailPrint "フォントを展開しています..."
    ; companion-build CI step deposits the three TTFs into dist/fonts/.
    ; Local makensis builds need them pre-staged in the same path.
    File "..\dist\fonts\InstrumentSerif-Regular.ttf"
    File "..\dist\fonts\Geist-Regular.ttf"
    File "..\dist\fonts\GeistMono-Regular.ttf"

    SetOutPath "$INSTDIR\config"
    DetailPrint "デフォルト設定を展開しています..."
    File "..\config\default.toml"

    SetOutPath "$INSTDIR\driver\${DRIVER_DIRNAME}"
    DetailPrint "SteamVR ドライバを展開しています..."
    File /r "..\driver\build\${DRIVER_DIRNAME}\*.*"

    SetOutPath "$INSTDIR"
    File "..\LICENSE"
    File "..\README.md"
    ; Third-party license disclosure (generated by `cargo about generate`).
    ; Built artifact lives next to the installer source so makensis can find
    ; it whether the build is local or in CI.
    File "..\THIRD_PARTY_LICENSES.html"

    ; -- SteamVR driver registration -----------------------------------------
    DetailPrint "SteamVR を検出しています..."
    Call RegisterSteamVRDriver

    ; -- Shortcuts -----------------------------------------------------------
    DetailPrint "ショートカットを作成しています..."
    CreateDirectory "$SMPROGRAMS\${APP_NAME}"
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" \
                   "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
    CreateShortcut "$SMPROGRAMS\${APP_NAME}\Uninstall.lnk" \
                   "$INSTDIR\Uninstall.exe" "" "$INSTDIR\Uninstall.exe" 0
    CreateShortcut "$DESKTOP\${APP_NAME}.lnk" \
                   "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0

    ; -- Uninstaller + registry ----------------------------------------------
    WriteUninstaller "$INSTDIR\Uninstall.exe"

    WriteRegStr HKLM "${REG_APP}" "InstallDir"      "$INSTDIR"
    WriteRegStr HKLM "${REG_APP}" "Version"         "${APP_VERSION}"

    ; Add/Remove Programs entry — minimum fields plus NoModify/NoRepair so
    ; Windows does not show greyed-out buttons for actions we don't implement.
    WriteRegStr   HKLM "${REG_UNINST}" "DisplayName"     "${APP_NAME}"
    WriteRegStr   HKLM "${REG_UNINST}" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
    WriteRegStr   HKLM "${REG_UNINST}" "QuietUninstallString" "$\"$INSTDIR\Uninstall.exe$\" /S"
    WriteRegStr   HKLM "${REG_UNINST}" "InstallLocation" "$INSTDIR"
    WriteRegStr   HKLM "${REG_UNINST}" "DisplayIcon"    "$INSTDIR\${APP_EXE}"
    WriteRegStr   HKLM "${REG_UNINST}" "DisplayVersion" "${APP_VERSION}"
    WriteRegStr   HKLM "${REG_UNINST}" "Publisher"      "${APP_PUBLISHER}"
    WriteRegStr   HKLM "${REG_UNINST}" "URLInfoAbout"   "${APP_URL}"
    WriteRegDWORD HKLM "${REG_UNINST}" "NoModify" 1
    WriteRegDWORD HKLM "${REG_UNINST}" "NoRepair" 1

    ; Estimated install size in KB for Add/Remove Programs display.
    ; $GetSize is provided by FileFunc.nsh.
    ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
    IntFmt $0 "0x%08X" $0
    WriteRegDWORD HKLM "${REG_UNINST}" "EstimatedSize" "$0"
SectionEnd

; ---- Function: SteamVR driver registration --------------------------------
;
; Resolves the SteamVR vrpathreg.exe path from the registry. Steam writes
; InstallPath into HKLM\Software\WOW6432Node\Valve\Steam on 64-bit Windows
; (and the same key without the WOW6432Node on the rare 32-bit hosts). If
; both lookups fail we fall back to the hard-coded default path so the
; common case still works.

Function RegisterSteamVRDriver
    Push $0
    Push $1
    Push $2

    SetRegView 32  ; Steam writes under Wow6432Node on 64-bit Windows
    ReadRegStr $0 HKLM "Software\Valve\Steam" "InstallPath"
    SetRegView default
    ${If} $0 == ""
        ReadRegStr $0 HKLM "Software\Valve\Steam" "InstallPath"
    ${EndIf}
    ${If} $0 == ""
        DetailPrint "Steam がレジストリに見つかりません — デフォルトパスを試行"
        StrCpy $0 "$PROGRAMFILES32\Steam"
    ${EndIf}

    StrCpy $1 "$0\steamapps\common\SteamVR\bin\win64\vrpathreg.exe"
    ${IfNot} ${FileExists} "$1"
        DetailPrint "vrpathreg が見つかりません: $1"
        DetailPrint "SteamVR をインストール後、手動で driver/install.bat を実行してください"
        Return
    ${EndIf}

    DetailPrint "ドライバを登録しています: $1"
    nsExec::ExecToStack '"$1" adddriver "$INSTDIR\driver\${DRIVER_DIRNAME}"'
    Pop $2  ; exit code
    ${If} $2 == 0
        DetailPrint "SteamVR ドライバ登録成功 — SteamVR を再起動してください"
    ${Else}
        DetailPrint "ドライバ登録失敗 (exit $2) — 手動で driver/install.bat を実行してください"
    ${EndIf}

    Pop $2
    Pop $1
    Pop $0
FunctionEnd

; ---- Function: SteamVR driver unregistration ------------------------------

Function un.UnregisterSteamVRDriver
    Push $0
    Push $1
    Push $2

    SetRegView 32
    ReadRegStr $0 HKLM "Software\Valve\Steam" "InstallPath"
    SetRegView default
    ${If} $0 == ""
        ReadRegStr $0 HKLM "Software\Valve\Steam" "InstallPath"
    ${EndIf}
    ${If} $0 == ""
        StrCpy $0 "$PROGRAMFILES32\Steam"
    ${EndIf}

    StrCpy $1 "$0\steamapps\common\SteamVR\bin\win64\vrpathreg.exe"
    ${If} ${FileExists} "$1"
        DetailPrint "SteamVR ドライバ登録を解除しています"
        nsExec::ExecToStack '"$1" removedriver "$INSTDIR\driver\${DRIVER_DIRNAME}"'
        Pop $2
        DetailPrint "vrpathreg removedriver exit: $2"
    ${EndIf}

    Pop $2
    Pop $1
    Pop $0
FunctionEnd

; ---- Section: uninstall ----------------------------------------------------

Section "Uninstall"
    ; Unregister SteamVR driver BEFORE deleting files — vrpathreg refuses
    ; missing paths but we'd rather it succeed first.
    Call un.UnregisterSteamVRDriver

    Delete "$INSTDIR\${APP_EXE}"
    Delete "$INSTDIR\LICENSE"
    Delete "$INSTDIR\README.md"
    Delete "$INSTDIR\THIRD_PARTY_LICENSES.html"
    Delete "$INSTDIR\Uninstall.exe"
    RMDir /r "$INSTDIR\fonts"
    RMDir /r "$INSTDIR\config"
    RMDir /r "$INSTDIR\driver"
    ; RMDir is non-recursive; $INSTDIR will only be removed if empty (i.e.
    ; the user hasn't dropped extra files into it).
    RMDir "$INSTDIR"

    Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
    Delete "$SMPROGRAMS\${APP_NAME}\Uninstall.lnk"
    RMDir  "$SMPROGRAMS\${APP_NAME}"
    Delete "$DESKTOP\${APP_NAME}.lnk"

    DeleteRegKey HKLM "${REG_APP}"
    DeleteRegKey HKLM "${REG_UNINST}"

    ; Intentionally NOT deleted:
    ;   %APPDATA%\FocusVisionPCVR\        (user config + lockout state)
    ;   %APPDATA%\FocusVisionPCVR\recordings\  (session recordings)
    ; Reinstall preserves these. Use Windows Storage settings to remove
    ; manually if a clean wipe is desired.
SectionEnd

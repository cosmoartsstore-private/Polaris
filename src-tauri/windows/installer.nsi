Unicode true
ManifestDPIAware true
; Windows 10 1607 以降向けに manifest の dpiAwareness を PerMonitorV2 にする。旧版は ManifestDPIAware true の設定を使う。
; NSIS の source tree の Docs に定義されている設定。参照先:
; https://github.com/kichik/nsis/blob/5fc0b87b819a9eec006df4967d08e522ddd651c9/Docs/src/attributes.but#L286-L300
; https://github.com/tauri-apps/tauri/pull/10106
ManifestDPIAwareness PerMonitorV2

!if "{{compression}}" == "none"
  SetCompress off
!else
  ; 既定の圧縮方式として LZMA を使用する。
  SetCompressor /SOLID "{{compression}}"
!endif

!include MUI2.nsh
!include FileFunc.nsh
!include x64.nsh
!include WordFunc.nsh
!include "utils.nsh"
!include "FileAssociation.nsh"
!include "Win\COM.nsh"
!include "Win\Propkey.nsh"
!include "StrFunc.nsh"
${StrCase}
${StrLoc}
${StrRep}

; $0の正規化済みpathを、既存rootの通常表記と8.3表記で境界まで比較する。
!macro IsInstallPathUnderRoot root result
  StrCpy ${result} 0
  ${StrCase} $1 "${root}" "U"
  StrLen $7 $1
  IntOp $7 $7 + 1
  StrCpy $7 $0 $7
  ${If} $0 == $1
  ${OrIf} $7 == "$1\"
    StrCpy ${result} 1
  ${Else}
    GetFullPathName /SHORT $8 "${root}"
    ${If} $8 != ""
      ${StrCase} $8 "$8" "U"
      StrLen $7 $8
      IntOp $7 $7 + 1
      StrCpy $7 $0 $7
      ${If} $0 == $8
      ${OrIf} $7 == "$8\"
        StrCpy ${result} 1
      ${EndIf}
    ${EndIf}
  ${EndIf}
!macroend

{{#if installer_hooks}}
!include "{{installer_hooks}}"
{{/if}}

!define WEBVIEW2APPGUID "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

!define MANUFACTURER "{{manufacturer}}"
!define PRODUCTNAME "{{product_name}}"
!define VERSION "{{version}}"
!define VERSIONWITHBUILD "{{version_with_build}}"
!define HOMEPAGE "{{homepage}}"
!define INSTALLMODE "{{install_mode}}"
!define LICENSE "{{license}}"
!define INSTALLERICON "{{installer_icon}}"
!define MAINBINARYNAME "{{main_binary_name}}"
!define MAINBINARYSRCPATH "{{main_binary_path}}"
!define BUNDLEID "{{bundle_id}}"
!define COPYRIGHT "{{copyright}}"
!define OUTFILE "{{out_file}}"
!define ARCH "{{arch}}"
!define ADDITIONALPLUGINSPATH "{{additional_plugins_path}}"
!define ALLOWDOWNGRADES "{{allow_downgrades}}"
!define DISPLAYLANGUAGESELECTOR "{{display_language_selector}}"
!define INSTALLWEBVIEW2MODE "{{install_webview2_mode}}"
!define WEBVIEW2INSTALLERARGS "{{webview2_installer_args}}"
!define WEBVIEW2BOOTSTRAPPERPATH "{{webview2_bootstrapper_path}}"
!define WEBVIEW2INSTALLERPATH "{{webview2_installer_path}}"
!define MINIMUMWEBVIEW2VERSION "{{minimum_webview2_version}}"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCTNAME}"
!define MANUKEY "Software\${MANUFACTURER}"
!define MANUPRODUCTKEY "${MANUKEY}\${PRODUCTNAME}"
!define UNINSTALLERSIGNCOMMAND "{{uninstaller_sign_cmd}}"
!define ESTIMATEDSIZE "{{estimated_size}}"
!define STARTMENUFOLDER "{{start_menu_folder}}"

Var PassiveMode
Var UpdateMode
Var NoShortcutMode
Var WixMode
Var OldMainBinaryName

Name "${PRODUCTNAME}"
BrandingText "${COPYRIGHT}"
OutFile "${OUTFILE}"

; この値は既定のインストール先には使用しない。
; directory 選択画面で product 名のフォルダを付加するために使用する。
; https://nsis.sourceforge.io/Reference/InstallDir
!define PLACEHOLDER_INSTALL_DIR "placeholder\${PRODUCTNAME}"
InstallDir "${PLACEHOLDER_INSTALL_DIR}"

VIProductVersion "${VERSIONWITHBUILD}"
VIAddVersionKey "ProductName" "${PRODUCTNAME}"
VIAddVersionKey "FileDescription" "${PRODUCTNAME}"
VIAddVersionKey "LegalCopyright" "${COPYRIGHT}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"

# 追加 plugin
!addplugindir "${ADDITIONALPLUGINSPATH}"

; アンインストーラーの署名 command
!if "${UNINSTALLERSIGNCOMMAND}" != ""
  !uninstfinalize '${UNINSTALLERSIGNCOMMAND}'
!endif

; perUser、perMachine、both の install mode を処理する。
!if "${INSTALLMODE}" == "perMachine"
  RequestExecutionLevel admin
!endif

!if "${INSTALLMODE}" == "currentUser"
  RequestExecutionLevel user
!endif

!if "${INSTALLMODE}" == "both"
  !define MULTIUSER_MUI
  !define MULTIUSER_INSTALLMODE_INSTDIR "${PRODUCTNAME}"
  !define MULTIUSER_INSTALLMODE_COMMANDLINE
  !if "${ARCH}" == "x64"
    !define MULTIUSER_USE_PROGRAMFILES64
  !else if "${ARCH}" == "arm64"
    !define MULTIUSER_USE_PROGRAMFILES64
  !endif
  !define MULTIUSER_INSTALLMODE_DEFAULT_REGISTRY_KEY "${UNINSTKEY}"
  !define MULTIUSER_INSTALLMODE_DEFAULT_REGISTRY_VALUENAME "CurrentUser"
  !define MULTIUSER_INSTALLMODEPAGE_SHOWUSERNAME
  !define MULTIUSER_INSTALLMODE_FUNCTION RestorePreviousInstallLocation
  !define MULTIUSER_EXECUTIONLEVEL Highest
  !include MultiUser.nsh
!endif

; インストーラーのアイコン
!if "${INSTALLERICON}" != ""
  !define MUI_ICON "${INSTALLERICON}"
!endif

; インストーラーの言語を保存する registry key
!define MUI_LANGDLL_REGISTRY_ROOT "HKCU"
!define MUI_LANGDLL_REGISTRY_KEY "${MANUPRODUCTKEY}"
!define MUI_LANGDLL_REGISTRY_VALUENAME "Installer Language"

; インストーラーの画面は表示順に定義する。
; 1. 開始画面
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_WELCOME

; 2. license 画面（定義がある場合）
!if "${LICENSE}" != ""
  !define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
  !insertmacro MUI_PAGE_LICENSE "${LICENSE}"
!endif

; 3. install mode 選択（both の場合）
!if "${INSTALLMODE}" == "both"
  !define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
  !insertmacro MULTIUSER_PAGE_INSTALLMODE
!endif

; 4. 再インストールとアンインストールの選択
;    既存のインストールを検出した場合だけ表示する。
Var ReinstallPageCheck
Page custom PageReinstall PageLeaveReinstall
Function PageReinstall
  ; WiX による既存のインストールがある場合は削除する。
  ;
  ; WiX はインストール情報を UUID ごとの registry key に保存する。
  ; 次の key 配下を列挙して対象を探す。
  ; `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall`
  ; DisplayName と Publisher が ${PRODUCTNAME} と ${MANUFACTURER} に一致するか確認する。
  ;
  ; 同じ product 名と publisher を持つ別のインストールが一致する可能性がある。
  ; この WiX installer 由来とは限らないため、
  ; 削除は利用者への確認を経て実行し、
  ; 対象が異なる場合は中止できるようにする。
  StrCpy $0 0
  wix_loop:
    EnumRegKey $1 HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall" $0
    StrCmp $1 "" wix_loop_done ; Exit loop if there is no more keys to loop on
    IntOp $0 $0 + 1
    ReadRegStr $R0 HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$1" "DisplayName"
    ReadRegStr $R1 HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$1" "Publisher"
    StrCmp "$R0$R1" "${PRODUCTNAME}${MANUFACTURER}" 0 wix_loop
    ReadRegStr $R0 HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$1" "UninstallString"
    ${StrCase} $R1 $R0 "L"
    ${StrLoc} $R0 $R1 "msiexec" ">"
    StrCmp $R0 0 0 wix_loop_done
    StrCpy $WixMode 1
    StrCpy $R6 "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\$1"
    Goto compare_version
  wix_loop_done:

  ; 既存のインストールがなければ再インストール画面を省略する。
  ReadRegStr $R0 SHCTX "${UNINSTKEY}" ""
  ReadRegStr $R1 SHCTX "${UNINSTKEY}" "UninstallString"
  ${IfThen} "$R0$R1" == "" ${|} Abort ${|}

  ; installer と既存アプリの version を比較し、
  ; 比較結果に応じて案内文を切り替える。
  compare_version:
  StrCpy $R4 "$(older)"
  ${If} $WixMode = 1
    ReadRegStr $R0 HKLM "$R6" "DisplayVersion"
  ${Else}
    ReadRegStr $R0 SHCTX "${UNINSTKEY}" "DisplayVersion"
  ${EndIf}
  ${IfThen} $R0 == "" ${|} StrCpy $R4 "$(unknown)" ${|}

  nsis_tauri_utils::SemverCompare "${VERSION}" $R0
  Pop $R0
  ; 同じ version の再インストール
  ${If} $R0 = 0
    StrCpy $R1 "$(alreadyInstalledLong)"
    StrCpy $R2 "$(addOrReinstall)"
    StrCpy $R3 "$(uninstallApp)"
    !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$(chooseMaintenanceOption)"
  ; 更新
  ${ElseIf} $R0 = 1
    StrCpy $R1 "$(olderOrUnknownVersionInstalled)"
    StrCpy $R2 "$(uninstallBeforeInstalling)"
    StrCpy $R3 "$(dontUninstall)"
    !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$(choowHowToInstall)"
  ; 旧 version への変更
  ${ElseIf} $R0 = -1
    StrCpy $R1 "$(newerVersionInstalled)"
    StrCpy $R2 "$(uninstallBeforeInstalling)"
    !if "${ALLOWDOWNGRADES}" == "true"
      StrCpy $R3 "$(dontUninstall)"
    !else
      StrCpy $R3 "$(dontUninstallDowngrade)"
    !endif
    !insertmacro MUI_HEADER_TEXT "$(alreadyInstalled)" "$(choowHowToInstall)"
  ${Else}
    Abort
  ${EndIf}

  ; passive mode では画面を省略する。
  ;
  ; 画面の省略判定は、既存 version と更新方向に関する
  ; 変数を設定した後に行う。
  ; そのため、関数の開始直後には省略しない。
  ;
  ${If} $PassiveMode = 1
    Call PageLeaveReinstall
  ${Else}
    nsDialogs::Create 1018
    Pop $R4
    ${IfThen} $(^RTL) = 1 ${|} nsDialogs::SetRTL $(^RTL) ${|}

    ${NSD_CreateLabel} 0 0 100% 24u $R1
    Pop $R1

    ${NSD_CreateRadioButton} 30u 50u -30u 8u $R2
    Pop $R2
    ${NSD_OnClick} $R2 PageReinstallUpdateSelection

    ${NSD_CreateRadioButton} 30u 70u -30u 8u $R3
    Pop $R3
    ; 旧 version への変更が禁止されている場合は選択肢を無効にする。
    !if "${ALLOWDOWNGRADES}" == "false"
      ${IfThen} $R0 = -1 ${|} EnableWindow $R3 0 ${|}
    !endif
    ${NSD_OnClick} $R3 PageReinstallUpdateSelection

    ; 初回表示、または前回の表示で2番目の選択肢が選ばれていない場合は、
    ; 1番目の選択肢を選択状態にする。
    ;
    ${If} $ReinstallPageCheck <> 2
      SendMessage $R2 ${BM_SETCHECK} ${BST_CHECKED} 0
    ${Else}
      SendMessage $R3 ${BM_SETCHECK} ${BST_CHECKED} 0
    ${EndIf}

    ${NSD_SetFocus} $R2
    nsDialogs::Show
  ${EndIf}
FunctionEnd
Function PageReinstallUpdateSelection
  ${NSD_GetState} $R2 $R1
  ${If} $R1 == ${BST_CHECKED}
    StrCpy $ReinstallPageCheck 1
  ${Else}
    StrCpy $ReinstallPageCheck 2
  ${EndIf}
FunctionEnd
Function PageLeaveReinstall
  ${NSD_GetState} $R2 $R1

  ; WiX からの移行では必ず既存アプリを削除する。
  ${If} $WixMode = 1
    Goto reinst_uninstall
  ${EndIf}

  ; update mode では削除せずに更新する。
  ${If} $UpdateMode = 1
    Goto reinst_done
  ${EndIf}

  ; $R0 は version の比較結果（同じ: 0、更新: 1、旧版へ変更: -1）を持つ。
  ; $R1 は選択状態を持つ。
  ;   1: 1番目の選択肢
  ;   0: 2番目の選択肢
  ${If} $R0 = 0 ; Same version, proceed
    ${If} $R1 = 1              ; User chose to add/reinstall
      Goto reinst_done
    ${Else}                    ; User chose to uninstall
      Goto reinst_uninstall
    ${EndIf}
  ${ElseIf} $R0 = 1 ; Upgrading
    ${If} $R1 = 1              ; User chose to uninstall
      Goto reinst_uninstall
    ${Else}
      Goto reinst_done         ; User chose NOT to uninstall
    ${EndIf}
  ${ElseIf} $R0 = -1 ; Downgrading
    ${If} $R1 = 1              ; User chose to uninstall
      Goto reinst_uninstall
    ${Else}
      Goto reinst_done         ; User chose NOT to uninstall
    ${EndIf}
  ${EndIf}

  reinst_uninstall:
    HideWindow
    ClearErrors

    ${If} $WixMode = 1
      ReadRegStr $R1 HKLM "$R6" "UninstallString"
      ExecWait '$R1' $0
    ${Else}
      ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""
      ReadRegStr $R1 SHCTX "${UNINSTKEY}" "UninstallString"
      ${IfThen} $UpdateMode = 1 ${|} StrCpy $R1 "$R1 /UPDATE" ${|} ; append /UPDATE
      ${IfThen} $PassiveMode = 1 ${|} StrCpy $R1 "$R1 /P" ${|} ; append /P
      StrCpy $R1 "$R1 _?=$4" ; append uninstall directory
      ExecWait '$R1' $0
    ${EndIf}

    BringToFront

    ${IfThen} ${Errors} ${|} StrCpy $0 2 ${|} ; ExecWait failed, set fake exit code

    ${If} $0 <> 0
    ${OrIf} ${FileExists} "$INSTDIR\${MAINBINARYNAME}.exe"
      ; WiX の削除が中止された場合は選択画面へ戻る。
      ${If} $WixMode = 1
      ${AndIf} $0 = 1602
        Abort
      ${EndIf}

      ; NSIS の削除が中止された場合は選択画面へ戻る。
      ${If} $0 = 1
        Abort
      ${EndIf}

      ; その他の失敗ではエラーを表示して選択画面へ戻る。
      MessageBox MB_ICONEXCLAMATION "$(unableToUninstall)"
      Abort
    ${EndIf}
  reinst_done:
FunctionEnd

; 5. インストール先の選択
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_DIRECTORY

; 6. Start menu の shortcut 選択
Var AppStartMenuFolder
!if "${STARTMENUFOLDER}" != ""
  !define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
  !define MUI_STARTMENUPAGE_DEFAULTFOLDER "${STARTMENUFOLDER}"
!else
  !define MUI_PAGE_CUSTOMFUNCTION_PRE Skip
!endif
!insertmacro MUI_PAGE_STARTMENU Application $AppStartMenuFolder

; 7. インストールの実行
!insertmacro MUI_PAGE_INSTFILES

; 8. 完了画面
;
; インストール画面には失敗の調査に必要な情報があるため、
; 自動で完了画面へ移動しない。
!define MUI_FINISHPAGE_NOAUTOCLOSE
; 完了画面の README 表示ボタンを desktop shortcut 作成に使用する。
!define MUI_FINISHPAGE_SHOWREADME
!define MUI_FINISHPAGE_SHOWREADME_TEXT "$(createDesktop)"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateOrUpdateDesktopShortcut
; インストール後のアプリ起動を選択できるようにする。
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION RunMainBinary
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipIfPassive
!insertmacro MUI_PAGE_FINISH

Function RunMainBinary
  nsis_tauri_utils::RunAsUser "$INSTDIR\${MAINBINARYNAME}.exe" ""
FunctionEnd

; アンインストーラーの画面
; 1. 削除の確認
!define MUI_PAGE_CUSTOMFUNCTION_PRE un.SkipIfPassive
!insertmacro MUI_UNPAGE_CONFIRM

; 2. 削除の実行
!insertmacro MUI_UNPAGE_INSTFILES

;言語
{{#each languages}}
!insertmacro MUI_LANGUAGE "{{this}}"
{{/each}}
!insertmacro MUI_RESERVEFILE_LANGDLL
{{#each language_files}}
  !include "{{this}}"
{{/each}}

Function .onInit
  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/NS" $NoShortcutMode
  ${IfNot} ${Errors}
    StrCpy $NoShortcutMode 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}

  !if "${DISPLAYLANGUAGESELECTOR}" == "true"
    !insertmacro MUI_LANGDLL_DISPLAY
  !endif

  !insertmacro SetContext

  ${If} $INSTDIR == "${PLACEHOLDER_INSTALL_DIR}"
    ; 既定のインストール先を設定する。
    !if "${INSTALLMODE}" == "perMachine"
      ${If} ${RunningX64}
        !if "${ARCH}" == "x64"
          StrCpy $INSTDIR "$PROGRAMFILES64\CosmoArtsStore\${PRODUCTNAME}"
        !else if "${ARCH}" == "arm64"
          StrCpy $INSTDIR "$PROGRAMFILES64\CosmoArtsStore\${PRODUCTNAME}"
        !else
          StrCpy $INSTDIR "$PROGRAMFILES\CosmoArtsStore\${PRODUCTNAME}"
        !endif
      ${Else}
        StrCpy $INSTDIR "$PROGRAMFILES\CosmoArtsStore\${PRODUCTNAME}"
      ${EndIf}
    !else if "${INSTALLMODE}" == "currentUser"
      StrCpy $INSTDIR "$LOCALAPPDATA\CosmoArtsStore\${PRODUCTNAME}"
    !endif

    Call RestorePreviousInstallLocation
  ${EndIf}


  !if "${INSTALLMODE}" == "both"
    !insertmacro MULTIUSER_INIT
  !endif
FunctionEnd

Function .onVerifyInstDir
  ${If} $INSTDIR == ""
    Return
  ${EndIf}

  ; 相対要素と区切りを正規化し、同じprefixを持つ別folderを誤って拒否しない。
  ${StrRep} $7 "$INSTDIR" "/" "\"
  IntOp $8 ${NSIS_MAX_STRLEN} * 2
  System::Alloc $8
  Pop $8
  ${If} $8 == 0
    MessageBox MB_ICONSTOP|MB_OK "インストール先のパスを解決できません。別のフォルダを選択してください。"
    Abort
  ${EndIf}
  System::Call 'kernel32::GetFullPathNameW(w r7, i ${NSIS_MAX_STRLEN}, p r8, p 0) i .r9'
  ${If} $9 == 0
  ${OrIf} $9 >= ${NSIS_MAX_STRLEN}
    System::Free $8
    MessageBox MB_ICONSTOP|MB_OK "インストール先のパスを解決できません。別のフォルダを選択してください。"
    Abort
  ${EndIf}
  System::Call '*$8(&w${NSIS_MAX_STRLEN} .r0)'
  System::Free $8
  ${StrCase} $0 "$0" "U"
  !insertmacro IsInstallPathUnderRoot "$PROGRAMFILES" $2
  ${If} $2 == 1
    MessageBox MB_ICONSTOP|MB_OK "このアプリは Program Files 配下へはインストールできません。LocalAppData 配下を選択してください。"
    Abort
  ${EndIf}

  ${If} ${RunningX64}
    !insertmacro IsInstallPathUnderRoot "$PROGRAMFILES64" $4
    ${If} $4 == 1
      MessageBox MB_ICONSTOP|MB_OK "このアプリは Program Files 配下へはインストールできません。LocalAppData 配下を選択してください。"
      Abort
    ${EndIf}
  ${EndIf}

  !insertmacro IsInstallPathUnderRoot "$WINDIR" $6
  ${If} $6 == 1
    MessageBox MB_ICONSTOP|MB_OK "Windows システム配下にはインストールできません。LocalAppData 配下を選択してください。"
    Abort
  ${EndIf}
FunctionEnd


Section EarlyChecks
  ; 旧 version への変更が禁止されている場合は silent mode の処理を中止する。
  !if "${ALLOWDOWNGRADES}" == "false"
  ${If} ${Silent}
    ; 旧 version へ変更する場合
    ${If} $R0 = -1
      System::Call 'kernel32::AttachConsole(i -1)i.r0'
      ${If} $0 <> 0
        System::Call 'kernel32::GetStdHandle(i -11)i.r0'
        System::call 'kernel32::SetConsoleTextAttribute(i r0, i 0x0004)' ; set red color
        FileWrite $0 "$(silentDowngrades)"
      ${EndIf}
      Abort
    ${EndIf}
  ${EndIf}
  !endif

SectionEnd

Section WebView2
  ; WebView2 が導入済みなら、この処理を省略する。
  ${If} ${RunningX64}
    ReadRegStr $4 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${Else}
    ReadRegStr $4 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${EndIf}
  ${If} $4 == ""
    ReadRegStr $4 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\${WEBVIEW2APPGUID}" "pv"
  ${EndIf}

  ${If} $4 == ""
    ; WebView2 の導入
    ;
    ; 更新時は省略する。
    ${If} $UpdateMode <> 1
      !if "${INSTALLWEBVIEW2MODE}" == "downloadBootstrapper"
        Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        DetailPrint "$(webview2Downloading)"
        NSISdl::download "https://go.microsoft.com/fwlink/p/?LinkId=2124703" "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        Pop $0
        ${If} $0 == "success"
          DetailPrint "$(webview2DownloadSuccess)"
        ${Else}
          DetailPrint "$(webview2DownloadError)"
          Abort "$(webview2AbortError)"
        ${EndIf}
        StrCpy $6 "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        Goto install_webview2
      !endif

      !if "${INSTALLWEBVIEW2MODE}" == "embedBootstrapper"
        Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        File "/oname=$TEMP\MicrosoftEdgeWebview2Setup.exe" "${WEBVIEW2BOOTSTRAPPERPATH}"
        DetailPrint "$(installingWebview2)"
        StrCpy $6 "$TEMP\MicrosoftEdgeWebview2Setup.exe"
        Goto install_webview2
      !endif

      !if "${INSTALLWEBVIEW2MODE}" == "offlineInstaller"
        Delete "$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe"
        File "/oname=$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe" "${WEBVIEW2INSTALLERPATH}"
        DetailPrint "$(installingWebview2)"
        StrCpy $6 "$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe"
        Goto install_webview2
      !endif

      Goto webview2_done

      install_webview2:
        DetailPrint "$(installingWebview2)"
        ; $6 は WebView2 installer のパスを持つ。
        ExecWait "$6 ${WEBVIEW2INSTALLERARGS} /install" $1
        ${If} $1 = 0
          DetailPrint "$(webview2InstallSuccess)"
        ${Else}
          DetailPrint "$(webview2InstallError)"
          Abort "$(webview2AbortError)"
        ${EndIf}
      webview2_done:
    ${EndIf}
  ${Else}
    !if "${MINIMUMWEBVIEW2VERSION}" != ""
      ${VersionCompare} "${MINIMUMWEBVIEW2VERSION}" "$4" $R0
      ${If} $R0 = 1
        update_webview:
          DetailPrint "$(installingWebview2)"
          ${If} ${RunningX64}
            ReadRegStr $R1 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate" "path"
          ${Else}
            ReadRegStr $R1 HKLM "SOFTWARE\Microsoft\EdgeUpdate" "path"
          ${EndIf}
          ${If} $R1 == ""
            ReadRegStr $R1 HKCU "SOFTWARE\Microsoft\EdgeUpdate" "path"
          ${EndIf}
          ${If} $R1 != ""
            ; Chromium updater の仕様: https://source.chromium.org/chromium/chromium/src/+/main:docs/updater/user_manual.md
            ; 参照元: HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Microsoft EdgeWebView\ModifyPath
            ExecWait `"$R1" /install appguid=${WEBVIEW2APPGUID}&needsadmin=true` $1
            ${If} $1 = 0
              DetailPrint "$(webview2InstallSuccess)"
            ${Else}
              MessageBox MB_ICONEXCLAMATION|MB_ABORTRETRYIGNORE "$(webview2InstallError)" IDIGNORE ignore IDRETRY update_webview
              Quit
              ignore:
            ${EndIf}
          ${EndIf}
      ${EndIf}
    !endif
  ${EndIf}
SectionEnd

Section Install
  ClearErrors
  SetOutPath $INSTDIR
  ${If} ${Errors}
    Abort "インストール先フォルダを作成できませんでした。書き込み権限を確認してから再実行してください。"
  ${EndIf}

  !ifmacrodef NSIS_HOOK_PREINSTALL
    !insertmacro NSIS_HOOK_PREINSTALL
  !endif

  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  ; 主実行ファイルの配置
  ClearErrors
  File "${MAINBINARYSRCPATH}"
  ${If} ${Errors}
    Abort "アプリ本体を配置できませんでした。実行中の${PRODUCTNAME}を終了し、書き込み権限を確認してから再実行してください。"
  ${EndIf}

  ; resource の配置
  {{#each resources_dirs}}
    CreateDirectory "$INSTDIR\\{{this}}"
  {{/each}}
  {{#each resources}}
    File /a "/oname={{this.[1]}}" "{{no-escape @key}}"
  {{/each}}

  ; 外部実行ファイルの配置
  {{#each binaries}}
    File /a "/oname={{this}}" "{{no-escape @key}}"
  {{/each}}

  ; ファイル関連付けの登録
  {{#each file_associations as |association| ~}}
    {{#each association.ext as |ext| ~}}
       !insertmacro APP_ASSOCIATE "{{ext}}" "{{or association.name ext}}" "{{association-description association.description ext}}" "$INSTDIR\${MAINBINARYNAME}.exe,0" "Open with ${PRODUCTNAME}" "$INSTDIR\${MAINBINARYNAME}.exe $\"%1$\""
    {{/each}}
  {{/each}}

  ; deep link の登録
  {{#each deep_link_protocols as |protocol| ~}}
    ClearErrors
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}" "URL Protocol" ""
    ${If} ${Errors}
      Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
    ${EndIf}
    ClearErrors
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}" "" "URL:${BUNDLEID} protocol"
    ${If} ${Errors}
      Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
    ${EndIf}
    ClearErrors
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}\DefaultIcon" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"
    ${If} ${Errors}
      Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
    ${EndIf}
    ClearErrors
    WriteRegStr SHCTX "Software\Classes\\{{protocol}}\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
    ${If} ${Errors}
      Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
    ${EndIf}
  {{/each}}

  ; アンインストーラーの生成
  ClearErrors
  WriteUninstaller "$INSTDIR\uninstall.exe"
  ${If} ${Errors}
    Abort "インストール情報またはショートカットを更新できませんでした。使用中のファイルとアクセス権を確認してから再実行してください。"
  ${EndIf}

  ; 次回のインストールに使用する $INSTDIR を registry に保存する。
  ClearErrors
  WriteRegStr SHCTX "${MANUPRODUCTKEY}" "" $INSTDIR
  ${If} ${Errors}
    Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
  ${EndIf}

  !if "${INSTALLMODE}" == "both"
    ; 次回の更新または削除で使用する install mode を保存する。
    ;
    ClearErrors
    WriteRegStr SHCTX "${UNINSTKEY}" $MultiUser.InstallMode 1
    ${If} ${Errors}
      Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
    ${EndIf}
  !endif

  ; 主実行ファイル名が変わった場合は旧ファイルを削除する。
  ReadRegStr $OldMainBinaryName SHCTX "${UNINSTKEY}" "MainBinaryName"
  ${If} $OldMainBinaryName != ""
  ${AndIf} $OldMainBinaryName != "${MAINBINARYNAME}.exe"
    ClearErrors
    Delete "$INSTDIR\$OldMainBinaryName"
    ${If} ${Errors}
      Abort "インストール情報またはショートカットを更新できませんでした。使用中のファイルとアクセス権を確認してから再実行してください。"
    ${EndIf}
  ${EndIf}

  ; 次回の更新に使用する MAINBINARYNAME を保存する。
  ClearErrors
  WriteRegStr SHCTX "${UNINSTKEY}" "MainBinaryName" "${MAINBINARYNAME}.exe"
  ${If} ${Errors}
    Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
  ${EndIf}

  ; Windows のアプリ一覧への登録
  ClearErrors
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"
  ${If} ${Errors}
    Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
  ${EndIf}
  ClearErrors
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayIcon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\""
  ${If} ${Errors}
    Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
  ${EndIf}
  ClearErrors
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  ${If} ${Errors}
    Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
  ${EndIf}
  ClearErrors
  WriteRegStr SHCTX "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"
  ${If} ${Errors}
    Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
  ${EndIf}
  ClearErrors
  WriteRegStr SHCTX "${UNINSTKEY}" "InstallLocation" "$\"$INSTDIR$\""
  ${If} ${Errors}
    Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
  ${EndIf}
  ClearErrors
  WriteRegStr SHCTX "${UNINSTKEY}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  ${If} ${Errors}
    Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
  ${EndIf}
  ClearErrors
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoModify" "1"
  ${If} ${Errors}
    Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
  ${EndIf}
  ClearErrors
  WriteRegDWORD SHCTX "${UNINSTKEY}" "NoRepair" "1"
  ${If} ${Errors}
    Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
  ${EndIf}

  ${GetSize} "$INSTDIR" "/M=uninstall.exe /S=0K /G=0" $0 $1 $2
  IntOp $0 $0 + ${ESTIMATEDSIZE}
  IntFmt $0 "0x%08X" $0
  ClearErrors
  WriteRegDWORD SHCTX "${UNINSTKEY}" "EstimatedSize" "$0"
  ${If} ${Errors}
    Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
  ${EndIf}

  !if "${HOMEPAGE}" != ""
    ClearErrors
    WriteRegStr SHCTX "${UNINSTKEY}" "URLInfoAbout" "${HOMEPAGE}"
    ${If} ${Errors}
      Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
    ${EndIf}
    ClearErrors
    WriteRegStr SHCTX "${UNINSTKEY}" "URLUpdateInfo" "${HOMEPAGE}"
    ${If} ${Errors}
      Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
    ${EndIf}
    ClearErrors
    WriteRegStr SHCTX "${UNINSTKEY}" "HelpLink" "${HOMEPAGE}"
    ${If} ${Errors}
      Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
    ${EndIf}
  !endif

  ; Start menu の shortcut 作成
  !insertmacro MUI_STARTMENU_WRITE_BEGIN Application
    Call CreateOrUpdateStartMenuShortcut
  !insertmacro MUI_STARTMENU_WRITE_END

  ; silent / passive mode では完了画面を省略するため、
  ; ここで desktop shortcut を作成する。
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    Call CreateOrUpdateDesktopShortcut
  ${EndIf}

  !ifmacrodef NSIS_HOOK_POSTINSTALL
    !insertmacro NSIS_HOOK_POSTINSTALL
  !endif

  ; 追加: 共通命名規則に基づくregistry位置への書き込み
  ClearErrors
  WriteRegStr HKCU "Software\CosmoArtsStore\Polaris" "InstallLocation" "$INSTDIR"
  ${If} ${Errors}
    Abort "登録情報を保存できませんでした。アクセス権を確認してから再実行してください。"
  ${EndIf}

  ; passive mode ではこの画面を自動で閉じる。
  ${If} $PassiveMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd

Function .onInstSuccess
  ; アプリを起動する /R の判定は silent / passive mode だけで行う。
  ; GUI では完了画面の選択に従って起動する。
  ${If} $PassiveMode = 1
  ${OrIf} ${Silent}
    ${GetOptions} $CMDLINE "/R" $R0
    ${IfNot} ${Errors}
      ${GetOptions} $CMDLINE "/ARGS" $R0
      nsis_tauri_utils::RunAsUser "$INSTDIR\${MAINBINARYNAME}.exe" "$R0"
    ${EndIf}
  ${EndIf}
FunctionEnd

Function un.onInit
  !insertmacro SetContext

  !if "${INSTALLMODE}" == "both"
    !insertmacro MULTIUSER_UNINIT
  !endif

  !insertmacro MUI_UNGETLANGUAGE

  ${GetOptions} $CMDLINE "/P" $PassiveMode
  ${IfNot} ${Errors}
    StrCpy $PassiveMode 1
  ${EndIf}

  ${GetOptions} $CMDLINE "/UPDATE" $UpdateMode
  ${IfNot} ${Errors}
    StrCpy $UpdateMode 1
  ${EndIf}
FunctionEnd

; 削除済み登録の再試行は成功とし、残存するkey/valueへのアクセス失敗は中断する。
!macro VerifyUninstallRegistryDeletion root key kind value
  ${If} ${Errors}
    StrCpy $8 0x80000001
    !if "${root}" == "HKLM"
      StrCpy $8 0x80000002
    !else if "${root}" == "SHCTX"
      !if "${INSTALLMODE}" == "perMachine"
        StrCpy $8 0x80000002
      !else if "${INSTALLMODE}" == "both"
        ${If} $MultiUser.InstallMode == "AllUsers"
          StrCpy $8 0x80000002
        ${EndIf}
      !endif
    !endif
    !if "${ARCH}" == "x64"
      StrCpy $9 0x101
    !else
      StrCpy $9 1
    !endif
    System::Call 'advapi32::RegOpenKeyExW(p r8, w "${key}", i 0, i r9, *p .r7) i .r9'
    ${If} $9 == 0
      !if "${kind}" == "value"
        System::Call 'advapi32::RegQueryValueExW(p r7, w "${value}", p 0, p 0, p 0, p 0) i .r9'
      !endif
      System::Call 'advapi32::RegCloseKey(p r7)'
    ${EndIf}
    ${If} $9 != 2
      Abort "登録情報を削除できませんでした。アクセス権を確認してから再実行してください。"
    ${EndIf}
    ClearErrors
  ${EndIf}
!macroend

Section Uninstall

  !ifmacrodef NSIS_HOOK_PREUNINSTALL
    !insertmacro NSIS_HOOK_PREUNINSTALL
  !endif

  !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"

  ; アプリ本体と同梱ファイルの削除
  ; 主実行ファイルの削除
  ClearErrors
  Delete "$INSTDIR\${MAINBINARYNAME}.exe"
  ${If} ${Errors}
    Abort "削除対象を削除できませんでした。アプリを終了し、アクセス権を確認してから再実行してください。"
  ${EndIf}

  ; resource の削除
  {{#each resources}}
  ClearErrors
    Delete "$INSTDIR\\{{this.[1]}}"
  ${If} ${Errors}
    Abort "削除対象を削除できませんでした。アプリを終了し、アクセス権を確認してから再実行してください。"
  ${EndIf}
  {{/each}}

  ; 外部実行ファイルの削除
  {{#each binaries}}
  ClearErrors
    Delete "$INSTDIR\\{{this}}"
  ${If} ${Errors}
    Abort "削除対象を削除できませんでした。アプリを終了し、アクセス権を確認してから再実行してください。"
  ${EndIf}
  {{/each}}

  ; ファイル関連付けの解除
  {{#each file_associations as |association| ~}}
    {{#each association.ext as |ext| ~}}
      !insertmacro APP_UNASSOCIATE "{{ext}}" "{{or association.name ext}}"
    {{/each}}
  {{/each}}

  ; deep link の解除
  {{#each deep_link_protocols as |protocol| ~}}
    ReadRegStr $R7 SHCTX "Software\Classes\\{{protocol}}\shell\open\command" ""
    ${If} $R7 == "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
      ClearErrors
      DeleteRegKey SHCTX "Software\Classes\\{{protocol}}"
      !insertmacro VerifyUninstallRegistryDeletion SHCTX "Software\Classes\\{{protocol}}" key ""
    ${EndIf}
  {{/each}}


  ; アンインストーラーの削除

  {{#each resources_ancestors}}
  RMDir /REBOOTOK "$INSTDIR\\{{this}}"
  {{/each}}
  RMDir "$INSTDIR"

  ; 通常の削除時だけ shortcut を削除する。
  ${If} $UpdateMode <> 1
    !insertmacro DeleteAppUserModelId

    ; Start menu の shortcut 削除
    !insertmacro MUI_STARTMENU_GETFOLDER Application $AppStartMenuFolder
    !insertmacro IsShortcutTarget "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      !insertmacro UnpinShortcut "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
      ClearErrors
      Delete "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
      ${If} ${Errors}
        Abort "インストール情報またはショートカットを更新できませんでした。使用中のファイルとアクセス権を確認してから再実行してください。"
      ${EndIf}
      RMDir "$SMPROGRAMS\$AppStartMenuFolder"
    ${EndIf}
    !insertmacro IsShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      !insertmacro UnpinShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk"
      ClearErrors
      Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
      ${If} ${Errors}
        Abort "インストール情報またはショートカットを更新できませんでした。使用中のファイルとアクセス権を確認してから再実行してください。"
      ${EndIf}
    ${EndIf}

    ; desktop shortcut の削除
    !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 = 1
      !insertmacro UnpinShortcut "$DESKTOP\${PRODUCTNAME}.lnk"
      ClearErrors
      Delete "$DESKTOP\${PRODUCTNAME}.lnk"
      ${If} ${Errors}
        Abort "インストール情報またはショートカットを更新できませんでした。使用中のファイルとアクセス権を確認してから再実行してください。"
      ${EndIf}
    ${EndIf}
  ${EndIf}


  ; 必須の削除を先に完了し、失敗時は登録情報とuninstallerを保持する。
  ${If} $UpdateMode <> 1
    SetShellVarContext current
  ClearErrors
    RmDir /r "$INSTDIR\Data\logs"
  ${If} ${Errors}
    Abort "削除対象を削除できませんでした。アプリを終了し、アクセス権を確認してから再実行してください。"
  ${EndIf}
  ClearErrors
    Delete "$INSTDIR\info.log"
  ${If} ${Errors}
    Abort "削除対象を削除できませんでした。アプリを終了し、アクセス権を確認してから再実行してください。"
  ${EndIf}
  ClearErrors
    RmDir /r "$APPDATA\${BUNDLEID}"
  ${If} ${Errors}
    Abort "削除対象を削除できませんでした。アプリを終了し、アクセス権を確認してから再実行してください。"
  ${EndIf}
  ClearErrors
    RmDir /r "$LOCALAPPDATA\${BUNDLEID}"
  ${If} ${Errors}
    Abort "削除対象を削除できませんでした。アプリを終了し、アクセス権を確認してから再実行してください。"
  ${EndIf}
  ${EndIf}

  ; Windows のアプリ一覧への登録を削除する。
  !if "${INSTALLMODE}" == "both"
    ClearErrors
    DeleteRegKey SHCTX "${UNINSTKEY}"
    !insertmacro VerifyUninstallRegistryDeletion SHCTX "${UNINSTKEY}" key ""
  !else if "${INSTALLMODE}" == "perMachine"
    ClearErrors
    DeleteRegKey HKLM "${UNINSTKEY}"
    !insertmacro VerifyUninstallRegistryDeletion HKLM "${UNINSTKEY}" key ""
  !else
    ClearErrors
    DeleteRegKey HKCU "${UNINSTKEY}"
    !insertmacro VerifyUninstallRegistryDeletion HKCU "${UNINSTKEY}" key ""
  !endif

  ; 通常の削除時は HKCU Run key の ${PRODUCTNAME} を削除する。
  ; アンインストール後に自動起動しないようにする。
  ; 登録がない場合は処理を行わない。
  ; 更新時は自動起動設定を保持する。
  ${If} $UpdateMode <> 1
    ClearErrors
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
    !insertmacro VerifyUninstallRegistryDeletion HKCU "Software\Microsoft\Windows\CurrentVersion\Run" value "${PRODUCTNAME}"
  ${EndIf}

  ; 再生成できないログを持つ Data\archive は保持する。
  ; Polaris には DB と画面がないため、Data\db と Data\EBWebView の削除は不要である。
  ${If} $UpdateMode <> 1
    ClearErrors
    DeleteRegKey SHCTX "${MANUPRODUCTKEY}"
    !insertmacro VerifyUninstallRegistryDeletion SHCTX "${MANUPRODUCTKEY}" key ""
    DeleteRegKey /ifempty SHCTX "${MANUKEY}"
    ClearErrors
    DeleteRegValue HKCU "${MANUPRODUCTKEY}" "Installer Language"
    !insertmacro VerifyUninstallRegistryDeletion HKCU "${MANUPRODUCTKEY}" value "Installer Language"
    DeleteRegKey /ifempty HKCU "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty HKCU "${MANUKEY}"
    ClearErrors
    DeleteRegKey HKCU "Software\CosmoArtsStore\Polaris"
    !insertmacro VerifyUninstallRegistryDeletion HKCU "Software\CosmoArtsStore\Polaris" key ""
    SetShellVarContext current
    ; 旧配置でインストール先直下に作られたファイル
    ; ログ原本のコピーを保持するため、Data\archive は削除しない。
    ; archive を保持し、Data と $INSTDIR は空の場合だけ削除する。
    RmDir "$INSTDIR\Data"
    RmDir "$INSTDIR"
  ${EndIf}


  ; 再試行用の実行ファイルは必須削除と登録解除が完了してから削除する。
  ClearErrors
  Delete "$INSTDIR\uninstall.exe"
  ${If} ${Errors}
    Abort "削除対象を削除できませんでした。アプリを終了し、アクセス権を確認してから再実行してください。"
  ${EndIf}
  RMDir "$INSTDIR"

  !ifmacrodef NSIS_HOOK_POSTUNINSTALL
    !insertmacro NSIS_HOOK_POSTUNINSTALL
  !endif

  ; passive mode または更新時は自動で閉じる。
  ${If} $PassiveMode = 1
  ${OrIf} $UpdateMode = 1
    SetAutoClose true
  ${EndIf}
SectionEnd

Function RestorePreviousInstallLocation
  ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""
  StrCmp $4 "" +2 0
    StrCpy $INSTDIR $4
FunctionEnd

Function Skip
  Abort
FunctionEnd

Function SkipIfPassive
  ${IfThen} $PassiveMode = 1  ${|} Abort ${|}
FunctionEnd
Function un.SkipIfPassive
  ${IfThen} $PassiveMode = 1  ${|} Abort ${|}
FunctionEnd

Function CreateOrUpdateStartMenuShortcut
  ; 旧版では product 名を MAINBINARYNAME に使用していたため、
  ; 旧 shortcut の参照先を現在の MAINBINARYNAME に変更する。
  StrCpy $R0 0

  !insertmacro IsShortcutTarget "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\$OldMainBinaryName"
  Pop $0
  ${If} $0 = 1
    !insertmacro SetShortcutTarget "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro IsShortcutTarget "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 != 1
      Abort "ショートカットの参照先を更新できませんでした。使用中のファイルとアクセス権を確認してから再実行してください。"
    ${EndIf}
    StrCpy $R0 1
  ${EndIf}

  !insertmacro IsShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\$OldMainBinaryName"
  Pop $0
  ${If} $0 = 1
    !insertmacro SetShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro IsShortcutTarget "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 != 1
      Abort "ショートカットの参照先を更新できませんでした。使用中のファイルとアクセス権を確認してから再実行してください。"
    ${EndIf}
    StrCpy $R0 1
  ${EndIf}

  ${If} $R0 = 1
    Return
  ${EndIf}

  ; 更新時または shortcut 作成を無効にした場合は作成を省略する。
  ; ただし WiX からの移行では作成する。
  ${If} $WixMode = 0
    ${If} $UpdateMode = 1
    ${OrIf} $NoShortcutMode = 1
      Return
    ${EndIf}
  ${EndIf}

  !if "${STARTMENUFOLDER}" != ""
    ClearErrors
    CreateDirectory "$SMPROGRAMS\$AppStartMenuFolder"
    ${If} ${Errors}
      Abort "インストール情報またはショートカットを更新できませんでした。使用中のファイルとアクセス権を確認してから再実行してください。"
    ${EndIf}
    ClearErrors
    CreateShortcut "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    ${If} ${Errors}
      Abort "インストール情報またはショートカットを更新できませんでした。使用中のファイルとアクセス権を確認してから再実行してください。"
    ${EndIf}
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
  !else
    ClearErrors
    CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    ${If} ${Errors}
      Abort "インストール情報またはショートカットを更新できませんでした。使用中のファイルとアクセス権を確認してから再実行してください。"
    ${EndIf}
    !insertmacro SetLnkAppUserModelId "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  !endif
FunctionEnd

Function CreateOrUpdateDesktopShortcut
  ; 旧版では product 名を MAINBINARYNAME に使用していたため、
  ; 旧 shortcut の参照先を現在の MAINBINARYNAME に変更する。
  !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\$OldMainBinaryName"
  Pop $0
  ${If} $0 = 1
    !insertmacro SetShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    !insertmacro IsShortcutTarget "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
    Pop $0
    ${If} $0 != 1
      Abort "ショートカットの参照先を更新できませんでした。使用中のファイルとアクセス権を確認してから再実行してください。"
    ${EndIf}
    Return
  ${EndIf}

  ; 更新時または shortcut 作成を無効にした場合は作成を省略する。
  ; ただし WiX からの移行では作成する。
  ${If} $WixMode = 0
    ${If} $UpdateMode = 1
    ${OrIf} $NoShortcutMode = 1
      Return
    ${EndIf}
  ${EndIf}

  ClearErrors
  CreateShortcut "$DESKTOP\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
  ${If} ${Errors}
    Abort "インストール情報またはショートカットを更新できませんでした。使用中のファイルとアクセス権を確認してから再実行してください。"
  ${EndIf}
  !insertmacro SetLnkAppUserModelId "$DESKTOP\${PRODUCTNAME}.lnk"
FunctionEnd

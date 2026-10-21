!macro NSIS_HOOK_PREINSTALL
    nsExec::ExecToLog 'taskkill /F /IM "${MAINBINARYNAME}.exe"'
!macroend

!macro NSIS_HOOK_POSTINSTALL
    ClearErrors
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Polaris" '"$INSTDIR\${MAINBINARYNAME}.exe"'
    ${If} ${Errors}
      Abort "自動起動の登録を保存できませんでした。アクセス権を確認してから再実行してください。"
    ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
    nsExec::ExecToLog 'taskkill /F /IM "${MAINBINARYNAME}.exe"'
!macroend

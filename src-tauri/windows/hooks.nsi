!macro NSIS_HOOK_PREINSTALL
    nsExec::ExecToLog 'taskkill /F /IM "${MAINBINARYNAME}.exe"'
!macroend

!macro NSIS_HOOK_POSTINSTALL
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Polaris" '"$INSTDIR\${MAINBINARYNAME}.exe"'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
    nsExec::ExecToLog 'taskkill /F /IM "${MAINBINARYNAME}.exe"'
!macroend

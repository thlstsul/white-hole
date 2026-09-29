!macro WhiteHoleRegisterBrowser
  ; 可执行文件完整路径
  !define WH_EXE_PATH "$INSTDIR\${MAINBINARYNAME}.exe"
  ; StartMenuInternet 下的注册键名（需唯一）
  !define WH_REG_KEY "${MAINBINARYNAME}.exe"
  !define WH_APP_NAME "White Hole"
  !define WH_PROG_ID "WhiteHoleHTM"

  ; ---------- 1. StartMenuInternet ----------
  WriteRegStr HKCU "SOFTWARE\Clients\StartMenuInternet\${WH_REG_KEY}" "" "${WH_APP_NAME}"

  ; Capabilities：应用信息
  WriteRegStr HKCU "SOFTWARE\Clients\StartMenuInternet\${WH_REG_KEY}\Capabilities" "ApplicationName" "${WH_APP_NAME}"
  WriteRegStr HKCU "SOFTWARE\Clients\StartMenuInternet\${WH_REG_KEY}\Capabilities" "ApplicationDescription" "${WH_APP_NAME} Browser"
  WriteRegStr HKCU "SOFTWARE\Clients\StartMenuInternet\${WH_REG_KEY}\Capabilities" "ApplicationIcon" "$\"${WH_EXE_PATH}$\",0"

  ; Capabilities：URL 关联
  WriteRegStr HKCU "SOFTWARE\Clients\StartMenuInternet\${WH_REG_KEY}\Capabilities\URLAssociations" "http" "${WH_PROG_ID}"
  WriteRegStr HKCU "SOFTWARE\Clients\StartMenuInternet\${WH_REG_KEY}\Capabilities\URLAssociations" "https" "${WH_PROG_ID}"

  ; Capabilities：文件关联
  WriteRegStr HKCU "SOFTWARE\Clients\StartMenuInternet\${WH_REG_KEY}\Capabilities\FileAssociations" ".html" "${WH_PROG_ID}"
  WriteRegStr HKCU "SOFTWARE\Clients\StartMenuInternet\${WH_REG_KEY}\Capabilities\FileAssociations" ".htm" "${WH_PROG_ID}"

  ; 默认图标
  WriteRegStr HKCU "SOFTWARE\Clients\StartMenuInternet\${WH_REG_KEY}\DefaultIcon" "" "$\"${WH_EXE_PATH}$\",0"

  ; shell\open\command
  WriteRegStr HKCU "SOFTWARE\Clients\StartMenuInternet\${WH_REG_KEY}\shell\open\command" "" "$\"${WH_EXE_PATH}$\""

  ; ---------- 2. ProgID ----------
  WriteRegStr HKCR "${WH_PROG_ID}" "" "${WH_APP_NAME} HTML Document"
  WriteRegStr HKCR "${WH_PROG_ID}\DefaultIcon" "" "$\"${WH_EXE_PATH}$\",0"
  WriteRegStr HKCR "${WH_PROG_ID}\shell\open\command" "" "$\"${WH_EXE_PATH}$\" $\"%1$\""
  WriteRegStr HKCR "${WH_PROG_ID}\Application" "ApplicationIcon" "$\"${WH_EXE_PATH}$\",0"

  ; ---------- 3. RegisteredApplications ----------
  WriteRegStr HKCU "SOFTWARE\RegisteredApplications" "WhiteHole" "SOFTWARE\Clients\StartMenuInternet\${WH_REG_KEY}\Capabilities"

  !undef WH_EXE_PATH
  !undef WH_REG_KEY
  !undef WH_APP_NAME
  !undef WH_PROG_ID
!macroend

!macro WhiteHoleUnregisterBrowser
  ; 删除 StartMenuInternet 注册键
  DeleteRegKey HKCU "SOFTWARE\Clients\StartMenuInternet\${MAINBINARYNAME}.exe\Capabilities\URLAssociations"
  DeleteRegKey HKCU "SOFTWARE\Clients\StartMenuInternet\${MAINBINARYNAME}.exe\Capabilities\FileAssociations"
  DeleteRegKey HKCU "SOFTWARE\Clients\StartMenuInternet\${MAINBINARYNAME}.exe\Capabilities"
  DeleteRegKey HKCU "SOFTWARE\Clients\StartMenuInternet\${MAINBINARYNAME}.exe\DefaultIcon"
  DeleteRegKey HKCU "SOFTWARE\Clients\StartMenuInternet\${MAINBINARYNAME}.exe\shell\open\command"
  DeleteRegKey HKCU "SOFTWARE\Clients\StartMenuInternet\${MAINBINARYNAME}.exe\shell\open"
  DeleteRegKey HKCU "SOFTWARE\Clients\StartMenuInternet\${MAINBINARYNAME}.exe\shell"
  DeleteRegKey HKCU "SOFTWARE\Clients\StartMenuInternet\${MAINBINARYNAME}.exe"

  ; 删除 RegisteredApplications 值
  DeleteRegValue HKCU "SOFTWARE\RegisteredApplications" "WhiteHole"

  ; 删除 ProgID（子键需先于父键删除）
  DeleteRegKey HKCR "WhiteHoleHTM\shell\open\command"
  DeleteRegKey HKCR "WhiteHoleHTM\shell\open"
  DeleteRegKey HKCR "WhiteHoleHTM\shell"
  DeleteRegKey HKCR "WhiteHoleHTM\DefaultIcon"
  DeleteRegKey HKCR "WhiteHoleHTM\Application"
  DeleteRegKey HKCR "WhiteHoleHTM"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro WhiteHoleRegisterBrowser
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro WhiteHoleUnregisterBrowser
!macroend

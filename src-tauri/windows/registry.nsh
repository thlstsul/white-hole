!macro NSIS_HOOK_POSTINSTALL
  ; 设置 HKCR\WhiteHoleHTM\shell\open\command 的默认值
  WriteRegStr HKCR "WhiteHoleHTM\shell\open\command" "" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""

  ; 设置 HKCR\WhiteHoleHTM\Application 的 ApplicationIcon 值
  WriteRegStr HKCR "WhiteHoleHTM\Application" "ApplicationIcon" "$\"$INSTDIR\${MAINBINARYNAME}.exe$\",0"

  ; 设置 HTTP 和 HTTPS 协议关联的用户选择
  WriteRegStr HKCU "Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice" "ProgId" "WhiteHoleHTM"
  WriteRegStr HKCU "Software\Microsoft\Windows\Shell\Associations\UrlAssociations\http\UserChoice" "ProgId" "WhiteHoleHTM"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 删除 WhiteHoleHTM 相关的注册表键
  DeleteRegKey HKCR "WhiteHoleHTM"

  ; 删除用户协议关联选择
  DeleteRegValue HKCU "Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice" "ProgId"
  DeleteRegValue HKCU "Software\Microsoft\Windows\Shell\Associations\UrlAssociations\http\UserChoice" "ProgId"
!macroend

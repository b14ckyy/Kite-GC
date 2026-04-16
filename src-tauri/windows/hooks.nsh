; Kite Ground Control — NSIS Installer Hooks
; Handles AppData cleanup on uninstall (user choice)

!macro NSIS_HOOK_POSTUNINSTALL
  MessageBox MB_YESNO "Sollen alle Anwendungsdaten (Einstellungen, Kartencache) entfernt werden?$\n$\nRemove all application data (settings, map cache)?" IDYES _cleanup IDNO _skip
  _cleanup:
    ; Remove WebView2 data (localStorage, IndexedDB = settings + tile cache)
    RMDir /r "$APPDATA\${PRODUCTNAME}"
    RMDir /r "$LOCALAPPDATA\${PRODUCTNAME}"
    ; Also try the identifier-based path used by Tauri
    RMDir /r "$APPDATA\com.kitegc.app"
    RMDir /r "$LOCALAPPDATA\com.kitegc.app"
  _skip:
!macroend

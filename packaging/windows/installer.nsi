; C7 Windows Installer
; NSIS Modern UI 2

!include "MUI2.nsh"
!include "WinVer.nsh"

; ── Defines ─────────────────────────────────────────────────────────────────

; `REPO_ROOT` and `APP_VERSION` are injected by CI via `-D` flags on `makensis`.

!define APP_NAME    "C7"
!define APP_EXE     "c7_app.exe"
!define PUBLISHER   "sata1000000"
!define UNINST_KEY  "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"

; ── Installer Settings ───────────────────────────────────────────────────────

Name             "${APP_NAME}"
OutFile          "${REPO_ROOT}\dist\C7_Windows.exe"
InstallDir       "$LOCALAPPDATA\${APP_NAME}"
InstallDirRegKey HKCU "Software\${APP_NAME}" "InstallDir"
RequestExecutionLevel user
SetCompressor    /SOLID lzma
BrandingText     "${APP_NAME} ${APP_VERSION}"

; ── MUI Configuration ────────────────────────────────────────────────────────

!define MUI_ABORTWARNING
!define MUI_ICON    "${REPO_ROOT}\icon.ico"
!define MUI_UNICON  "${REPO_ROOT}\icon.ico"

; Header image
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADERIMAGE_BITMAP "${REPO_ROOT}\packaging\windows\assets\header.bmp"

; Finish page
!define MUI_FINISHPAGE_RUN          "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT     "Launch ${APP_NAME}"

; ── Minimum OS Check ─────────────────────────────────────────────────────────

; Crash out if not running on Windows 10 or newer.
; WinRT and UCRT64 require Windows 10+.

Function .onInit
  ${IfNot} ${AtLeastWin10}
    MessageBox MB_OK|MB_ICONSTOP "${APP_NAME} requires Windows 10 or later."
    Abort
  ${EndIf}
FunctionEnd

; ── Pages ────────────────────────────────────────────────────────────────────

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; ── Install Section ──────────────────────────────────────────────────────────

Section "${APP_NAME}" SecMain

  SetOutPath "$INSTDIR"
  File /r "${REPO_ROOT}\dist\C7_Windows\*"

  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; Start Menu shortcut
  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortcut  "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"

  ; Add/Remove Programs registry entry (`HKCU` = no admin required)
  WriteRegStr   HKCU "${UNINST_KEY}" "DisplayName"     "${APP_NAME}"
  WriteRegStr   HKCU "${UNINST_KEY}" "DisplayIcon"     "$INSTDIR\${APP_EXE}"
  WriteRegStr   HKCU "${UNINST_KEY}" "UninstallString" "$INSTDIR\Uninstall.exe"
  WriteRegStr   HKCU "${UNINST_KEY}" "DisplayVersion"  "${APP_VERSION}"
  WriteRegStr   HKCU "${UNINST_KEY}" "Publisher"       "${PUBLISHER}"
  WriteRegStr   HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify"        1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair"        1

SectionEnd

; ── Uninstall Section ────────────────────────────────────────────────────────

Section "Uninstall"

  RMDir /r "$INSTDIR"
  RMDir /r "$APPDATA\${APP_NAME}"
  RMDir /r "$SMPROGRAMS\${APP_NAME}"

  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\${APP_NAME}"

SectionEnd

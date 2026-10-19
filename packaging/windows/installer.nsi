; The Windows setup program, built by scripts/package.py with NSIS 3:
;
;   makensis -DVERSION=X.Y.Z -DSTAGE=<staged files> -DOUTFILE=<setup.exe>
;            -DSIZE_KB=<installed size> [-DSIGN_PY=<python> -DSIGN_SCRIPT=<package.py>]
;            packaging/windows/installer.nsi
;
; It installs the editor and audioc into Program Files, a Start-menu entry,
; the CLAP and VST3 plugins into Common Files (where hosts look), file types
; for projects, packs and blocks, and an uninstaller listed under Installed
; apps. `setup.exe /S` installs silently; `/D=<folder>` (last) picks the folder.

Unicode true
ManifestDPIAware true
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!include "x64.nsh"
!include "LogicLib.nsh"

!define NAME "Audio Constructor"
!define PUBLISHER "Audio Constructor contributors"
!define URL "https://github.com/Andorbal/audio-constructor"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\AudioConstructor"
!define PLUGIN "Audio Constructor"

!ifndef VERSION
  !error "Run this through scripts/package.py, which defines VERSION, STAGE, and OUTFILE."
!endif
!ifndef STAGE
  !error "STAGE (the staged files' folder) isn't defined."
!endif
!ifndef OUTFILE
  !error "OUTFILE (the setup program to write) isn't defined."
!endif
!ifndef SIZE_KB
  !define SIZE_KB 0
!endif

; Signing, when package.py has a certificate: the installer and the
; uninstaller it writes are signed as NSIS finishes each one.
!ifdef SIGN_PY
  !finalize '"${SIGN_PY}" "${SIGN_SCRIPT}" --sign-file "%1"' = 0
  !uninstfinalize '"${SIGN_PY}" "${SIGN_SCRIPT}" --sign-file "%1"' = 0
!endif

Name "${NAME}"
Caption "${NAME} ${VERSION} Setup"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${NAME}"
InstallDirRegKey HKLM "Software\${NAME}" "InstallDir"
RequestExecutionLevel admin
BrandingText "${NAME} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${NAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${NAME} ${VERSION} Setup"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "MIT License"

!define MUI_ICON "${STAGE}\icon.ico"
!define MUI_UNICON "${STAGE}\icon.ico"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_SMALLDESC
!define MUI_FINISHPAGE_RUN "$INSTDIR\audio-constructor.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Open ${NAME}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${STAGE}\LICENSE.txt"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "${NAME} needs 64-bit Windows." /SD IDOK
    Abort
  ${EndIf}
  SetRegView 64
  SetShellVarContext all
FunctionEnd

Function un.onInit
  SetRegView 64
  SetShellVarContext all
FunctionEnd

; A file type that opens in the editor.
!macro FileType EXT PROGID DESCRIPTION
  WriteRegStr HKLM "Software\Classes\${EXT}" "" "${PROGID}"
  WriteRegStr HKLM "Software\Classes\${PROGID}" "" "${DESCRIPTION}"
  WriteRegStr HKLM "Software\Classes\${PROGID}\DefaultIcon" "" "$INSTDIR\audio-constructor.exe,0"
  WriteRegStr HKLM "Software\Classes\${PROGID}\shell\open\command" "" '"$INSTDIR\audio-constructor.exe" "%1"'
!macroend

!macro RemoveFileType EXT PROGID
  ; Only if the type still points at us.
  ReadRegStr $0 HKLM "Software\Classes\${EXT}" ""
  ${If} $0 == "${PROGID}"
    DeleteRegKey HKLM "Software\Classes\${EXT}"
  ${EndIf}
  DeleteRegKey HKLM "Software\Classes\${PROGID}"
!macroend

Section "${NAME} and audioc" SecApp
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${STAGE}\audio-constructor.exe"
  File "${STAGE}\audioc.exe"
  File "${STAGE}\README.md"
  File "${STAGE}\LICENSE.txt"
  File "${STAGE}\CHANGELOG.md"
  File "${STAGE}\icon.ico"
  ; A new version's examples replace the old ones entirely.
  RMDir /r "$INSTDIR\examples"
  File /r "${STAGE}\examples"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${NAME}.lnk" "$INSTDIR\audio-constructor.exe"

  !insertmacro FileType ".acproj" "AudioConstructor.Project" "${NAME} project"
  !insertmacro FileType ".acpack" "AudioConstructor.Pack" "${NAME} pack"
  !insertmacro FileType ".acblock" "AudioConstructor.Block" "${NAME} block"
  ; Tell Explorer the associations changed (SHCNE_ASSOCCHANGED).
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'

  WriteRegStr HKLM "Software\${NAME}" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "${NAME}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "URLInfoAbout" "${URL}"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\audio-constructor.exe,0"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "EstimatedSize" ${SIZE_KB}
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "CLAP plugins" SecClap
  SetOutPath "$COMMONFILES64\CLAP"
  File "${STAGE}\plugins\${PLUGIN}.clap"
SectionEnd

Section "VST3 plugins" SecVst3
  SetOutPath "$COMMONFILES64\VST3"
  RMDir /r "$COMMONFILES64\VST3\${PLUGIN}.vst3"
  File /r "${STAGE}\plugins\${PLUGIN}.vst3"
SectionEnd

Section /o "Desktop shortcut" SecDesktop
  CreateShortcut "$DESKTOP\${NAME}.lnk" "$INSTDIR\audio-constructor.exe"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecApp} "The editor, the audioc command-line tool, and the examples."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecClap} "Audio Constructor Effect and Instrument, for hosts that load CLAP plugins."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecVst3} "Audio Constructor Effect and Instrument, for hosts that load VST3 plugins."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "A shortcut to the editor on the desktop."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  Delete "$INSTDIR\audio-constructor.exe"
  Delete "$INSTDIR\audioc.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE.txt"
  Delete "$INSTDIR\CHANGELOG.md"
  Delete "$INSTDIR\icon.ico"
  RMDir /r "$INSTDIR\examples"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${NAME}.lnk"
  Delete "$DESKTOP\${NAME}.lnk"
  Delete "$COMMONFILES64\CLAP\${PLUGIN}.clap"
  RMDir /r "$COMMONFILES64\VST3\${PLUGIN}.vst3"

  !insertmacro RemoveFileType ".acproj" "AudioConstructor.Project"
  !insertmacro RemoveFileType ".acpack" "AudioConstructor.Pack"
  !insertmacro RemoveFileType ".acblock" "AudioConstructor.Block"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'

  DeleteRegKey HKLM "Software\${NAME}"
  DeleteRegKey HKLM "${UNINSTALL_KEY}"
SectionEnd

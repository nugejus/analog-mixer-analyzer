; makensis -DVERSION=x.y.z packaging/installer.nsi  (build-time paths are relative to this file)
Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user
!include "MUI2.nsh"

Name "Mixer Analyzer"
OutFile "../dist/MixerAnalyzer-Setup.exe"
InstallDir "$LOCALAPPDATA\Programs\MixerAnalyzer"

!define UNKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\MixerAnalyzer"
!define MUI_FINISHPAGE_RUN "$INSTDIR\MixerAnalyzer.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Run Mixer Analyzer"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section
  SetOutPath "$INSTDIR"
  File "../dist/MixerAnalyzer.exe"
  File "../README.md"
  File "../assets/Pretendard-LICENSE.txt"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  ; 0.5.0 and older made Korean-named shortcuts: remove them on upgrade.
  Delete "$SMPROGRAMS\믹서 분석기.lnk"
  Delete "$DESKTOP\믹서 분석기.lnk"
  CreateShortCut "$SMPROGRAMS\Mixer Analyzer.lnk" "$INSTDIR\MixerAnalyzer.exe"
  CreateShortCut "$DESKTOP\Mixer Analyzer.lnk" "$INSTDIR\MixerAnalyzer.exe"
  WriteRegStr HKCU "${UNKEY}" "DisplayName" "Mixer Analyzer"
  WriteRegStr HKCU "${UNKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNKEY}" "DisplayIcon" "$INSTDIR\MixerAnalyzer.exe"
  WriteRegStr HKCU "${UNKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNKEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegDWORD HKCU "${UNKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNKEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\MixerAnalyzer.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\Pretendard-LICENSE.txt"
  Delete "$INSTDIR\Uninstall.exe"
  Delete "$SMPROGRAMS\Mixer Analyzer.lnk"
  Delete "$DESKTOP\Mixer Analyzer.lnk"
  ; Korean-named shortcuts of 0.5.0 and older.
  Delete "$SMPROGRAMS\믹서 분석기.lnk"
  Delete "$DESKTOP\믹서 분석기.lnk"
  DeleteRegKey HKCU "${UNKEY}"
  RMDir "$INSTDIR"
SectionEnd

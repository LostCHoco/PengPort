; PengPort NSIS installer hooks.
;
; 이 파일은 tauri.conf.json 의 bundle.windows.nsis.installerHooks 가 가리킨다. Tauri 의
; NSIS template 이 매크로를 이 파일에서 찾는다.
;
; ## 0.1.6 부터 추가 — PengdollPark 잔재 자동 정리
;
; PengPort 의 옛 productName/identifier 변경 history:
;
;   0.1.0~0.1.2  productName=PengdollPark, identifier=app.pengdollpark
;   0.1.3~       productName=PengPort,     identifier=PengPort
;
; 0.1.0~0.1.2 옛 버전을 갖고 있던 PC 에서 0.1.3+ setup.exe 실행 시 NSIS 가 옛 PengdollPark
; 의 흔적을 detect 해서 "기존 제거 후 설치" 흐름 trigger. 그러나 옛 uninstaller 가 손상
; (수동 삭제 / 디스크 이동 등) 됐을 때 "제거할 수 없습니다" 에러 → 새 PengPort 설치 막힘.
;
; 이 preInstall hook 이 그 잔재를 강제로 정리하고 새 설치 진행:
;   1. 옛 PengdollPark uninstaller 가 있으면 silent 실행 시도 (best-effort, 결과 무시)
;   2. registry 의 옛 UninstallString 키 강제 삭제
;   3. 옛 install dir (Program Files\PengdollPark 등) 강제 삭제
;
; user data (`%APPDATA%\PengdollPark` 등) 는 건드리지 않는다 — 사용자가 0.1.5 에서 [PengPort
; 삭제] 로 명시 정리하든가, 새 PengPort 가 그대로 두는 게 안전.

!macro NSIS_HOOK_PREINSTALL
  DetailPrint "옛 PengdollPark 잔재 확인 중..."

  ; 1. HKCU 의 PengdollPark uninstaller silent 실행 시도 (있으면).
  ;    `_?=$INSTDIR` 없이 호출 — 우리는 옛 install dir 어디 있는지 모르고, 어차피 그 다음
  ;    단계에서 강제 RMDir 한다. 이 호출은 best-effort.
  ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PengdollPark" "UninstallString"
  StrCmp $R0 "" hkcu_done
    DetailPrint "옛 PengdollPark uninstaller 실행 시도 (HKCU)..."
    ; 따옴표 포함된 path 일 수 있어 그대로 넘긴다. 실패해도 진행.
    ExecWait '$R0 /S'
  hkcu_done:

  ; 2. HKLM (machine-wide install).
  ReadRegStr $R0 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PengdollPark" "UninstallString"
  StrCmp $R0 "" hklm_done
    DetailPrint "옛 PengdollPark uninstaller 실행 시도 (HKLM)..."
    ExecWait '$R0 /S'
  hklm_done:

  ; 3. registry 의 옛 키 강제 삭제. uninstaller 실패해도 stale 키가 남으면 다음 install 도
  ;    같은 흐름 trigger 되니 정리 필수.
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\PengdollPark"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PengdollPark"
  DeleteRegKey HKLM "Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\PengdollPark"

  ; 4. 옛 install dir 강제 삭제. uninstaller 가 못 지웠거나 폴더만 남은 케이스.
  ;    PengPort 새 dir ($PROGRAMFILES64\PengPort) 와 충돌하지 않으니 안전.
  RMDir /r "$PROGRAMFILES64\PengdollPark"
  RMDir /r "$PROGRAMFILES\PengdollPark"
  RMDir /r "$LOCALAPPDATA\Programs\PengdollPark"

  DetailPrint "PengdollPark 잔재 정리 완료."
!macroend

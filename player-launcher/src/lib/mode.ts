// PengPort 사용 모드 — normal vs ephemeral (1회용).
//
// ## 설계
//
// 사용자가 첫 실행 시 dialog 로 선택. localStorage 에 저장:
//
// - **normal**: 일반 모드. 평소 쓰는 내 PC — 라이브러리 / third-party 앱 계정 영구 저장.
//   다음 launch 부터 자동 로드.
//   localStorage 에 `pengport.mode = "normal"` 저장 → 다음부터 selector 안 묻기.
//
// - **ephemeral**: 공용 PC (PC방 등). 종료 시 모든 데이터 + PengPort 자체 자동 정리.
//   localStorage 에 저장 안 함 — ephemeral 종료 시 어차피 wipe 되니 무관. 같은 PC 에서
//   다음 launch 시 또 selector 표시 (의도된 동작 — 매번 새 install 후 선택).
//
// ## 저장 위치
//
// localStorage 만 사용 — keyring 은 시크릿용이라 mode 저장에 과함. file 기반보다 단순.
// ephemeral 모드 자체가 종료 시 localStorage 전체 wipe 되므로 잔재 무관.

const LS_MODE = "pengport.mode";

export type Mode = "normal" | "ephemeral";

/**
 * 현재 모드 조회.
 *
 * - `"normal"` / `"ephemeral"` — 사용자가 명시 선택한 값
 * - `null` — 미설정 (첫 실행). selector dialog 표시 trigger.
 */
export function getMode(): Mode | null {
  const raw = localStorage.getItem(LS_MODE);
  if (raw === "normal" || raw === "ephemeral") return raw;
  return null;
}

/**
 * 모드 저장.
 *
 * normal 만 영구 저장. ephemeral 은 의도적으로 저장 안 함 — 다음 launch 시 또 selector
 * 표시 (매번 1회용 의식적 선택).
 */
export function setMode(mode: Mode): void {
  if (mode === "normal") {
    localStorage.setItem(LS_MODE, mode);
  }
  // ephemeral 은 저장 안 함
}

/** 사용자가 [내 PC] 선택했었는지. UI 분기에 사용. */
export function isModeSelected(): boolean {
  return getMode() !== null;
}

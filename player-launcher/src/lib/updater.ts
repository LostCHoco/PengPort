// 자동 업데이트 wrapper.
//
// Tauri 2 의 updater 플러그인을 감싸 프론트 어디서든 동일 API 로 호출.
// 다운로드/설치는 서명 검증 실패 시 자동 거부됨 (공개키 불일치 → throw).
//
// PSP 정신상 client (software) 는 instance-agnostic — public 다운로드.
// 별도 Bearer 토큰 필요 없음 (instance 접근만 EVENTS_TOKEN 으로 보호).

import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export interface UpdateInfo {
  available: boolean;
  version: string | null;
  currentVersion: string | null;
  body: string | null;
  /** 실제 다운로드 + 설치 + 재실행 트리거. 호출자가 사용자 동의 후 부르기. */
  install: (() => Promise<void>) | null;
}

/**
 * 서버에서 업데이트를 체크한다.
 * 네트워크/서명 문제가 있으면 throw.
 */
export async function checkForUpdate(): Promise<UpdateInfo> {
  const update: Update | null = await check();

  if (!update || !update.available) {
    return {
      available: false,
      version: null,
      currentVersion: update?.currentVersion ?? null,
      body: null,
      install: null,
    };
  }

  return {
    available: true,
    version: update.version,
    currentVersion: update.currentVersion,
    body: update.body ?? null,
    install: async () => {
      await update.downloadAndInstall();
      await relaunch();
    },
  };
}

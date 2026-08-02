// 자동 업데이트 wrapper.
//
// PengPort 는 portable exe(인스톨러 없음) 라 `@tauri-apps/plugin-updater`의 JS API
// (`downloadAndInstall`)를 그대로 못 쓴다 — 그 install() 은 다운로드한 바이트를 NSIS/MSI
// 인스톨러 실행 파일로 간주하고 spawn 하기 때문. 대신 Rust 쪽에서 그 크레이트의
// `download()`(서명 검증까지 끝난 바이트 반환, install() 과 분리된 API)만 재사용하고
// "설치"는 rename-to-delete 로 직접 구현한 커맨드(`check_self_update`/
// `install_self_update`, `commands/self_update.rs`)를 호출한다. 서명 검증은 여전히
// `tauri.conf.json`의 기존 pubkey 로 자동 수행 — 실패하면 커맨드 자체가 throw.
//
// PSP 정신상 client (software) 는 instance-agnostic — public 다운로드.
// 별도 Bearer 토큰 필요 없음 (instance 접근만 EVENTS_TOKEN 으로 보호).

import { invoke } from "@tauri-apps/api/core";

export interface UpdateInfo {
  available: boolean;
  version: string | null;
  currentVersion: string | null;
  body: string | null;
  /** 실제 다운로드 + 교체 + 재시작 트리거. 호출자가 사용자 동의 후 부르기. */
  install: (() => Promise<void>) | null;
}

interface SelfUpdateInfo {
  version: string;
  current_version: string;
  body: string | null;
}

/**
 * 서버에서 업데이트를 체크한다.
 * 네트워크/서명 문제가 있으면 throw.
 */
export async function checkForUpdate(): Promise<UpdateInfo> {
  const update = await invoke<SelfUpdateInfo | null>("check_self_update");

  if (!update) {
    return {
      available: false,
      version: null,
      currentVersion: null,
      body: null,
      install: null,
    };
  }

  return {
    available: true,
    version: update.version,
    currentVersion: update.current_version,
    body: update.body,
    install: async () => {
      // 성공하면 프로세스가 곧 종료되므로 이 Promise 는 resolve 안 될 수도 있음(정상).
      await invoke<void>("install_self_update");
    },
  };
}

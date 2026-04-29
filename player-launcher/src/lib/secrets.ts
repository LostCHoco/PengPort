// OS keychain 기반 시크릿 wrapper.
//
// Rust 의 `commands::secrets` (keyring crate) 를 통해 OS native secret store
// (Windows Credential Manager / macOS Keychain / Linux Secret Service) 사용.
// localStorage 보다 강한 격리 — user session 단위 보호 + 디스크 평문 차단.
//
// multi-instance 모델: keyring account 이름이 'instance_token:<instance_id>' 로
// instance 별 격리. instance list 자체는 lib/instances.ts 에서 관리.

import { invoke } from "@tauri-apps/api/core";

export const instanceToken = {
  /** 특정 instance 의 bearer 토큰 조회. 없으면 null. */
  load: (instanceId: string) =>
    invoke<string | null>("instance_token_load", { instanceId }),
  /** 특정 instance 의 bearer 토큰 저장 (빈 문자열이면 자동 clear). */
  save: (instanceId: string, token: string) =>
    invoke<void>("instance_token_save", { instanceId, token }),
  /** 특정 instance 의 bearer 토큰 삭제. */
  clear: (instanceId: string) =>
    invoke<void>("instance_token_clear", { instanceId }),
};

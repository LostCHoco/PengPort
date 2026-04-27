// OS keychain 기반 시크릿 wrapper.
//
// Rust 의 `commands::secrets` (keyring crate) 를 통해 OS native secret store
// (Windows Credential Manager / macOS Keychain / Linux Secret Service) 사용.
// localStorage 보다 강한 격리 — user session 단위 보호 + 디스크 평문 차단.

import { invoke } from "@tauri-apps/api/core";

const LS_INSTANCE_TOKEN_LEGACY = "pengport.instance_token";

export const instanceToken = {
  /** keyring 에서 instance bearer 토큰 조회. 없으면 null. */
  load: () => invoke<string | null>("instance_token_load"),
  /** keyring 에 instance bearer 토큰 저장 (빈 문자열이면 자동 clear). */
  save: (token: string) => invoke<void>("instance_token_save", { token }),
  /** keyring 에서 instance bearer 토큰 삭제. */
  clear: () => invoke<void>("instance_token_clear"),
};

/**
 * keyring 에서 instance 토큰을 읽되, 옛 localStorage 값이 있으면 자동으로
 * keyring 으로 옮기고 localStorage 정리 (한 번만 작동).
 */
export async function loadInstanceTokenWithMigration(): Promise<string | null> {
  const fromKeyring = await instanceToken.load();
  if (fromKeyring) return fromKeyring;

  const legacy = localStorage.getItem(LS_INSTANCE_TOKEN_LEGACY);
  if (legacy) {
    await instanceToken.save(legacy);
    localStorage.removeItem(LS_INSTANCE_TOKEN_LEGACY);
    return legacy;
  }
  return null;
}

// 자동 업데이트 wrapper.
//
// Tauri 2 의 updater 플러그인을 감싸 프론트 어디서든 동일 API 로 호출.
// 다운로드/설치는 서명 검증 실패 시 자동 거부됨 (공개키 불일치 → throw).
//
// /updates 엔드포인트가 Bearer 토큰을 요구하므로, 매 요청마다 토큰을 Rust 측
// (get_update_token) 에서 가져와 헤더로 동봉한다. 이 헤더는 manifest 요청과
// installer 다운로드 양쪽에 모두 적용된다 (tauri-plugin-updater 의 동작).

import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { invoke } from "@tauri-apps/api/core";

export interface UpdateInfo {
  available: boolean;
  version: string | null;
  currentVersion: string | null;
  body: string | null;
  /** 실제 다운로드 + 설치 + 재실행 트리거. 호출자가 사용자 동의 후 부르기. */
  install: (() => Promise<void>) | null;
}

async function buildAuthHeaders(): Promise<Record<string, string>> {
  const token = await invoke<string>("get_update_token");
  if (!token) return {};
  return { Authorization: `Bearer ${token}` };
}

/**
 * 서버에서 업데이트를 체크한다.
 * 네트워크/서명/인증 문제가 있으면 throw.
 */
export async function checkForUpdate(): Promise<UpdateInfo> {
  const headers = await buildAuthHeaders();
  const update: Update | null = await check({ headers });

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

/** 토큰 출처: "saved" (사용자가 Settings 에서 입력) | "embedded" (빌드 임베드) | "none" (없음). */
export type TokenSource = "saved" | "embedded" | "none";

export async function getUpdateToken(): Promise<string> {
  return invoke<string>("get_update_token");
}

export async function setUpdateToken(token: string): Promise<void> {
  await invoke("set_update_token", { token });
}

/** 입력 토큰을 저장 전에 서버 ping 으로 검증. 잘못된 토큰이면 throw. */
export async function validateUpdateToken(token: string): Promise<void> {
  await invoke("validate_update_token", { token });
}

export async function getUpdateTokenSource(): Promise<TokenSource> {
  return invoke<TokenSource>("update_token_source");
}

/** UI 표시용 마스킹: 앞 6자 + 끝 4자만 노출. */
export function maskToken(token: string): string {
  if (!token) return "(없음)";
  if (token.length <= 12) return "•".repeat(token.length);
  return `${token.slice(0, 6)}••••${token.slice(-4)}`;
}

// Prism 탐지/설치 Tauri command wrapper.
//
// 이 파일은 프론트엔드의 (PSP 외) Tauri 호출 wrapper. PSP commands 는
// `@/lib/psp` (별도 모듈) 사용.
//
// 옛 servers.toml 흐름의 fetch_meta / ping_server / sync_instances /
// launch_server 는 PSP 단방향 마이그레이션과 함께 제거됨.

import { invoke } from "@tauri-apps/api/core";

export type PrismSource =
  | "user"
  | "env"
  | "portable"
  | "bundled"
  | "system"
  | "path";

export interface PrismLocation {
  exe: string;
  data_dir: string;
  source: PrismSource;
}

/** 시스템에 설치된/번들된 Prism 위치 탐지. 못 찾으면 null. */
export async function detectPrism(): Promise<PrismLocation | null> {
  return await invoke<PrismLocation | null>("detect_prism");
}

export interface PrismDownloadResult {
  version: string;
  install_dir: string;
}

/**
 * PrismLauncher Windows Portable 최신 release 를 다운받아
 * `%LOCALAPPDATA%\app.pengport\prism\` 에 설치한다. 30초 ~ 2분 소요.
 */
export async function downloadPrism(): Promise<PrismDownloadResult> {
  return await invoke<PrismDownloadResult>("download_prism");
}

/**
 * 사용자가 직접 폴더를 골라 Prism 위치를 강제 지정. 빈 문자열을 넘기면 해제.
 * 폴더 안에 prismlauncher.exe 가 없으면 throw. 갱신 후 재탐지 결과 반환.
 */
export async function setPrismOverride(
  root: string,
): Promise<PrismLocation | null> {
  return await invoke<PrismLocation | null>("set_prism_override", { root });
}

/** PengPort 가 다운로드한 전용 Prism (Bundled) 을 삭제. */
export async function removeBundledPrism(): Promise<PrismLocation | null> {
  return await invoke<PrismLocation | null>("remove_bundled_prism");
}

/** 실행 중인 PSP service (Prism + Minecraft tree) 를 강제 종료. */
export async function stopServer(serverId: string): Promise<void> {
  return await invoke<void>("stop_server", { serverId });
}

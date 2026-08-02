// third-party app 탐지/설정 + 데이터 정리/언인스톨 Tauri command wrapper.
//
// 라이브러리(레시피 목록 + 임포트 + 실행) 관련 커맨드는 `@/lib/library` 사용.

import { invoke } from "@tauri-apps/api/core";
import type { ThirdPartyAppDescriptor } from "@/lib/library";

// `pengport_shared::actions::ThirdPartyAppSource`(snake_case). 탐지/override
// 커맨드는 이제 app_id 하나로 등록된 모든 third-party app 을 다룬다 — Prism 전용
// 이름(`detect_prism` 등)은 남아있지 않음(`docs/design/THIRD_PARTY_PLATFORM_MODEL.md`).
export type ThirdPartyAppSource = "user_override" | "bundled" | "system";

// `pengport_shared::actions::ResolvedThirdPartyApp`.
export interface ThirdPartyAppLocation {
  exe: string;
  data_root: string;
  source: ThirdPartyAppSource;
}

/** 등록된 third-party app id+표시 이름 목록(로컬 파일 `%APPDATA%\PengPort\
 * third_party_apps.json`) — 설정 화면(`ThirdPartyApps.tsx`)이 카드를 몇 개 그릴지 결정.
 * `supports_download`는 자동 다운로드 버튼 노출 여부. */
export interface ThirdPartyAppSummary {
  id: string;
  label: string;
  supports_download: boolean;
}

export async function listThirdPartyApps(): Promise<ThirdPartyAppSummary[]> {
  return await invoke<ThirdPartyAppSummary[]>("list_third_party_apps");
}

/** 등록된 third-party app descriptor 전체(모든 필드) — 편집 다이얼로그가 기존 값을
 * 채워 넣는 데 사용. `listThirdPartyApps`(요약)와 달리 편집에 필요한 모든 필드를 포함. */
export async function listThirdPartyAppDescriptors(): Promise<ThirdPartyAppDescriptor[]> {
  return await invoke<ThirdPartyAppDescriptor[]>("list_third_party_app_descriptors");
}

/** 시스템에 설치된/override 지정된/번들된 위치를 탐지. 못 찾으면 null. */
export async function detectThirdPartyApp(appId: string): Promise<ThirdPartyAppLocation | null> {
  return await invoke<ThirdPartyAppLocation | null>("detect_third_party_app", { appId });
}

/**
 * 사용자가 직접 폴더를 골라 위치를 강제 지정. 빈 문자열을 넘기면 해제.
 * 폴더 안에 대상 앱의 실행 파일이 없으면 throw. 갱신 후 재탐지 결과 반환.
 */
export async function configureThirdPartyAppOverride(
  appId: string,
  root: string,
): Promise<ThirdPartyAppLocation | null> {
  return await invoke<ThirdPartyAppLocation | null>("configure_third_party_app_override", { appId, root });
}

/** PengPort 가 다운로드한 전용 사본(Bundled) 을 삭제. */
export async function removeBundledThirdPartyApp(appId: string): Promise<ThirdPartyAppLocation | null> {
  return await invoke<ThirdPartyAppLocation | null>("remove_bundled_third_party_app", { appId });
}

/** 서드파티 앱 descriptor 직접 추가/갱신 — 설정 화면(`ThirdPartyApps.tsx`)의 등록/편집
 * 폼이 호출. `library_upsert`(레시피)의 대응. */
export async function thirdPartyAppUpsert(descriptor: ThirdPartyAppDescriptor): Promise<void> {
  return await invoke<void>("third_party_app_upsert", { descriptor });
}

/** 서드파티 앱 descriptor 삭제. `library_remove`(레시피)의 대응. */
export async function thirdPartyAppRemove(id: string): Promise<boolean> {
  return await invoke<boolean>("third_party_app_remove", { id });
}

export interface ThirdPartyAppDownloadResult {
  /** GitHub release 태그 — `static_url` 전략(release 개념 없음)이면 null. */
  version: string | null;
  install_dir: string;
}

/**
 * 등록된 third-party app 을 descriptor 의 `download_strategy`로 받아 전용 사본
 * (`%LOCALAPPDATA%\PengPort\<app_id>\`)에 설치한다. 30초 ~ 2분 소요. app_id 하나로
 * 모든 앱을 다루는 범용 커맨드 — 앱별 프론트 함수를 따로 등록할 필요 없음(옛
 * `downloadPrism` 전용 커맨드 폐지, `docs/design/THIRD_PARTY_PLATFORM_MODEL.md` §3).
 */
export async function downloadThirdPartyApp(appId: string): Promise<ThirdPartyAppDownloadResult> {
  return await invoke<ThirdPartyAppDownloadResult>("download_third_party_app", { appId });
}

/** 실행 중인 third-party app 인스턴스(예: Prism + Minecraft tree) 를 강제 종료. */
export async function stopServer(serverId: string): Promise<void> {
  return await invoke<void>("stop_server", { serverId });
}

/** 해당 service 의 Prism 인스턴스가 현재 실행 중인지. */
export async function isServiceRunning(serviceId: string): Promise<boolean> {
  return await invoke<boolean>("is_service_running", { serviceId });
}

// ============================================================
// 데이터 정리/언인스톨 (위험 작업 — frontend 에서 사용자 confirm 후 호출)
// ============================================================

/** PengPort 가 만든 third-party app 인스턴스 폴더 1개 삭제. */
export async function removeThirdPartyAppInstance(appId: string, instanceId: string): Promise<void> {
  return await invoke<void>("remove_third_party_app_instance", { appId, instanceId });
}

export interface WipeReport {
  keyring_cleared: number;
  paths_removed: string[];
  failures: string[];
}

/** 특정 third-party app(`appId`)의 인스턴스 폴더들(`instanceIds`)을 정리 대상으로. */
export interface ThirdPartyAppInstanceWipeTarget {
  appId: string;
  instanceIds: string[];
}

/**
 * PengPort 가 만든 모든 native state 초기화.
 * - keyring 의 instance_token:* (req.instanceIds)
 * - PengPort 가 만든 third-party app 인스턴스들 (req.thirdPartyAppInstances, app_id 별 그룹)
 * - app_data_root 의 trust.json / third_party_app_overrides.json
 * - app_cache_root 의 bundled third-party app(예: prism) 사본들
 *
 * 호출 후 frontend 가 localStorage 도 정리해야 한다 (이 함수는 native 만 담당).
 */
export async function wipeAllData(req: {
  instanceIds: string[];
  thirdPartyAppInstances: ThirdPartyAppInstanceWipeTarget[];
}): Promise<WipeReport> {
  return await invoke<WipeReport>("wipe_all_data", {
    req: {
      instance_ids: req.instanceIds,
      third_party_app_instances: req.thirdPartyAppInstances.map((t) => ({
        app_id: t.appId,
        instance_ids: t.instanceIds,
      })),
    },
  });
}

/**
 * exe + data 폴더를 지우고 자체 종료. 반환 안 됨. kiosk(ephemeral) 모드 종료 자동
 * cleanup 흐름 전용 — portable 모델이라 인스톨러가 없어 대응하는 수동 UI는 없다.
 */
export async function uninstallSelf(): Promise<void> {
  return await invoke<void>("uninstall_self");
}

// PSP Tauri command wrapper.
//
// 모든 호출은 src-tauri/src/commands/psp.rs 의 #[tauri::command] 와 1:1 대응.
// 변경 시 양쪽 동시 갱신.

import { invoke } from "@tauri-apps/api/core";
import type {
  ActionOutcome,
  InstanceMetadata,
  ServiceManifest,
  ServicesCatalog,
  TrustEntryDto,
} from "./types";

// ============================================================
// Fetch
// ============================================================

/** `<instance_url>/.well-known/pengport-instance` GET. */
export async function pspLoadInstance(
  instanceUrl: string,
): Promise<InstanceMetadata> {
  return await invoke<InstanceMetadata>("psp_load_instance", {
    instanceUrl,
  });
}

/** `<service_url>/.well-known/pengport-service` GET. */
export async function pspLoadManifest(
  serviceUrl: string,
  bearerToken?: string,
): Promise<ServiceManifest> {
  return await invoke<ServiceManifest>("psp_load_manifest", {
    serviceUrl,
    bearerToken: bearerToken ?? null,
  });
}

/**
 * Catalog URL GET. instance metadata 의 `endpoints.catalog` 사용.
 * JSON / TOML 자동 감지 (백엔드).
 */
export async function pspLoadCatalog(
  catalogUrl: string,
  bearerToken?: string,
): Promise<ServicesCatalog> {
  return await invoke<ServicesCatalog>("psp_load_catalog", {
    catalogUrl,
    bearerToken: bearerToken ?? null,
  });
}

/**
 * 초대 코드 redeem (invite B). 안정적 INVITE_CODE 를 인스턴스의 현재 EVENTS_TOKEN 으로
 * 교환. 토큰은 링크/URL 에 없고 이 호출로만 받음 → 사용자는 토큰을 보지 않음.
 * 실패(코드 불일치/redeem 비활성) 시 throw.
 */
export async function pspRedeemInvite(
  instanceUrl: string,
  code: string,
): Promise<string> {
  return await invoke<string>("invite_redeem", { instanceUrl, code });
}

// ============================================================
// Validate
// ============================================================

/** Manifest 일관성 검증 (actions ⊆ permissions, URL allowlist 등). 실패 시 throw. */
export async function pspValidateManifest(
  manifest: ServiceManifest,
  baseUrl: string,
  catalogId?: string,
): Promise<void> {
  return await invoke<void>("psp_validate_manifest", {
    manifest,
    baseUrl,
    catalogId: catalogId ?? null,
  });
}

// ============================================================
// Invoke
// ============================================================

export interface InvokeActionInput {
  /** "open_url" | "open_protocol" | "submit_form" | "native_third_party_app" */
  kind: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  args: any;
  /** Service URL (origin 비교 기준). */
  manifestOrigin: string;
  /** manifest.permissions.external_urls. */
  externalUrls: string[];
  /** third_party 분기에서만 의미. service id 그대로 전달 권고. */
  instanceId?: string;
}

/**
 * Action dispatch + 검증 + OS 호출.
 *
 * 결과 분기:
 * - `done`/`submitted`/`launched` → 완료
 * - `needs_confirm` → 사용자 동의 필요. ConsentDialog 띄우고 동의 시
 *   `pspTrust(...)` 호출 → 다시 `pspInvokeAction(...)` 재시도.
 */
export async function pspInvokeAction(
  input: InvokeActionInput,
): Promise<ActionOutcome> {
  return await invoke<ActionOutcome>("psp_invoke_action", {
    kind: input.kind,
    args: input.args,
    manifestOrigin: input.manifestOrigin,
    externalUrls: input.externalUrls,
    instanceId: input.instanceId ?? null,
  });
}

export interface SubmitFormWithDataInput {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  args: any;
  fieldValues: Record<string, unknown>;
  manifestOrigin: string;
  externalUrls: string[];
  bearerToken?: string;
}

export async function pspSubmitFormWithData(
  input: SubmitFormWithDataInput,
): Promise<ActionOutcome> {
  return await invoke<ActionOutcome>("psp_submit_form_with_data", {
    args: input.args,
    fieldValues: input.fieldValues,
    manifestOrigin: input.manifestOrigin,
    externalUrls: input.externalUrls,
    bearerToken: input.bearerToken ?? null,
  });
}

// ============================================================
// Trust (TOFU)
// ============================================================

/** 사용자가 NeedsConfirm 에 동의했을 때 호출. 같은 (kind, id) 면 갱신. */
export async function pspTrust(input: {
  trustKind: string;
  subjectId: string;
  display: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  metadata: any;
}): Promise<void> {
  return await invoke<void>("psp_trust", {
    trustKind: input.trustKind,
    subjectId: input.subjectId,
    display: input.display,
    metadata: input.metadata,
  });
}

/** 신뢰 철회. 다음 invoke 시 NeedsConfirm 으로 돌아감. */
export async function pspRevokeTrust(
  trustKind: string,
  subjectId: string,
): Promise<boolean> {
  return await invoke<boolean>("psp_revoke_trust", {
    trustKind,
    subjectId,
  });
}

/** 신뢰 목록. Settings 의 "신뢰 관리" UI 용. */
export async function pspListTrusts(
  kindFilter?: string,
): Promise<TrustEntryDto[]> {
  return await invoke<TrustEntryDto[]>("psp_list_trusts", {
    kindFilter: kindFilter ?? null,
  });
}

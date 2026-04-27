// PSP (PengPort Service Protocol) 타입 정의.
//
// Rust 측 (`shared/src/psp/`, `src-tauri/src/commands/psp.rs`) 와 수동 동기화.
// 변경 시 `cargo test -p pengport-shared` 로 Rust 측 테스트 통과 + 이 파일 갱신.
//
// 명세: `docs/spec/psp-v1.md`.

// ============================================================
// Instance metadata (`/.well-known/pengport-instance`)
// ============================================================

export type InstanceAuthType = "none" | "token" | "oauth2";

export interface OAuth2Endpoints {
  authorization_url: string;
  token_url: string;
  scopes: string[];
}

export interface InstanceAuth {
  type: InstanceAuthType;
  /** `type=token` 시 사용자에게 보여줄 안내 문구. */
  token_hint?: string;
  /** `type=oauth2` 시 OAuth endpoints. */
  oauth2?: OAuth2Endpoints;
}

export interface InstanceEndpoints {
  /** services.toml 또는 services.d/ URL. */
  catalog: string;
  /** broadcaster SSE URL (선택). */
  events?: string;
}

export interface OperatorInfo {
  name: string;
  contact?: string;
}

export interface InstanceMetadata {
  schema_version: number;
  name: string;
  description?: string;
  operator: OperatorInfo;
  endpoints: InstanceEndpoints;
  auth: InstanceAuth;
  icon_url?: string;
  pengport_min_version?: string;
  /** Phase 3+ instance fingerprint pinning. */
  public_key_fingerprint?: string;
}

// ============================================================
// Services catalog (services.toml or services.d/)
// ============================================================

export interface ServiceHint {
  name?: string;
  icon?: string;
}

export interface ServiceEntry {
  id: string;
  /** 항상 "psp" (현재 한 종류만). */
  type: string;
  url: string;
  enabled: boolean;
  hint?: ServiceHint;
}

export interface InstanceInfo {
  display_name: string;
  description?: string;
}

export interface ServicesCatalog {
  /** 항상 "2" (PSP v1). */
  schema_version: string;
  instance?: InstanceInfo;
  services: ServiceEntry[];
}

// ============================================================
// Service manifest (`/.well-known/pengport-service`)
// ============================================================

export type CategoryHint =
  | "game"
  | "media"
  | "files"
  | "communication"
  | "dev"
  | "infra"
  | "productivity"
  | "other";

export type NativeActionKind =
  | "open_url"
  | "open_protocol"
  | "submit_form"
  | "native_third_party_app";

export type EventType = "status_changed" | "notification" | "custom";

export interface Permissions {
  native_actions: NativeActionKind[];
  /** glob 패턴 (예: "https://example.com/*"). */
  external_urls: string[];
  events: EventType[];
}

/** Service action — `kind` 별 `args` schema 가 다름. */
export interface ServiceAction {
  id: string;
  label: string;
  primary?: boolean;
  kind: NativeActionKind;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  args: any;
}

export interface ManifestEndpoints {
  status: string;
  events?: string;
}

export interface ServiceManifest {
  schema_version: number;
  id: string;
  name: string;
  description?: string;
  icon_url?: string;
  category_hint?: CategoryHint;
  endpoints: ManifestEndpoints;
  actions: ServiceAction[];
  permissions: Permissions;
  psp_version: number;
}

// ============================================================
// Status response (manifest.endpoints.status)
// ============================================================

export type MetricType =
  | "number"
  | "percentage"
  | "bytes"
  | "timestamp"
  | "string"
  | "players";

export interface Metric {
  id: string;
  label: string;
  type: MetricType;
  /** `type` 별 의미 다름. `players` 면 `{online, max, names}` 객체. */
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  value: any;
}

export type BadgeLevel = "info" | "warning" | "error";

export interface Badge {
  id: string;
  label: string;
  level: BadgeLevel;
}

export interface StatusResponse {
  online: boolean;
  metrics: Metric[];
  badges: Badge[];
  last_updated?: string;
}

/** `metric.type === "players"` 일 때 `metric.value` 의 형태. */
export interface PlayersMetricValue {
  online: number;
  max: number;
  names: string[];
}

// ============================================================
// Events (SSE)
// ============================================================

export type NotificationLevel = "info" | "warning" | "error";

export interface NotificationEvent {
  level: NotificationLevel;
  title: string;
  body?: string;
}

export interface CustomEvent {
  event_type: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  payload: any;
}

/** Service 가 직접 발행하는 SSE event. */
export type ServiceEvent =
  | { event: "status_changed"; data: StatusResponse }
  | { event: "notification"; data: NotificationEvent }
  | { event: "custom"; data: CustomEvent };

/** Broadcaster 가 multiplexing 하는 SSE event. service_id 항상 포함. */
export type InstanceEvent =
  | {
      event: "service_status_changed";
      service_id: string;
      status: StatusResponse;
    }
  | {
      event: "service_notification";
      service_id: string;
      notification: NotificationEvent;
    }
  | {
      event: "service_custom";
      service_id: string;
      event_type: string;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      payload: any;
    };

// ============================================================
// ActionOutcome (Tauri command 응답)
// ============================================================

/** `psp_invoke_action` 의 응답. */
export type ActionOutcome =
  | { kind: "done" }
  | { kind: "submitted"; status: number }
  | { kind: "launched"; instance_id: string }
  | {
      kind: "needs_confirm";
      trust_kind: string;
      subject_id: string;
      display: string;
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      details: any;
    };

// ============================================================
// Trust store
// ============================================================

export interface TrustEntryDto {
  subject_kind: string;
  subject_id: string;
  display: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  metadata: any;
  /** UNIX seconds. */
  trusted_at: number;
}

// Generic PSP service 카드.
//
// manifest 만 받아 actions / status / metrics / badges 를 렌더링.
// 카테고리 (MC/Media/...) 무관 — manifest 가 표현하는 모든 service 가 동일 컴포넌트.
//
// status polling: 30초 간격 + 페이지 visible 일 때 즉시 fetch.
// SSE 구독은 Phase 2 (현 PSP v1 의 service-level events 인증 미정).
//
// invoke 책임은 부모 (PspLibrary) — confirm 재시도 / outcome 분기 / 에러 표시 등이
// 한 곳에 모이도록. 이 카드는 status 표시 + action 클릭 콜백만.

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import type {
  Badge as PspBadge,
  Metric,
  PlayersMetricValue,
  ServiceAction,
  ServiceManifest,
  StatusResponse,
} from "@/lib/psp";

type StatusState =
  | { kind: "loading" }
  | { kind: "ok"; status: StatusResponse }
  | { kind: "error"; message: string };

interface Props {
  manifest: ServiceManifest;
  /** Authorization header 용 (없으면 미전송). status 와 action 양쪽 사용. */
  bearerToken?: string;
  /** Catalog hint (manifest 보다 우선 표시할 이름/아이콘 — 보통은 manifest 사용). */
  hintName?: string;
  hintIcon?: string;
  /** action 클릭. 부모가 invoke + confirm 재시도 / 결과 처리. */
  onAction: (action: ServiceAction) => void;
  /** 부모가 알리는 invoke 진행 중 action id (해당 버튼 비활성). null = idle. */
  invokingActionId?: string | null;
}

const STATUS_POLL_INTERVAL_MS = 30_000;

export function ServiceCard({
  manifest,
  bearerToken,
  hintName,
  hintIcon,
  onAction,
  invokingActionId = null,
}: Props) {
  const [statusState, setStatusState] = useState<StatusState>({ kind: "loading" });

  const displayName = hintName ?? manifest.name;
  const iconUrl = hintIcon ?? manifest.icon_url;

  // Status polling — visibility 기반.
  useEffect(() => {
    let cancelled = false;

    const fetchStatus = async () => {
      try {
        const headers: HeadersInit = { Accept: "application/json" };
        if (bearerToken) headers.Authorization = `Bearer ${bearerToken}`;
        const resp = await fetch(manifest.endpoints.status, { headers });
        if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
        const data = (await resp.json()) as StatusResponse;
        if (!cancelled) setStatusState({ kind: "ok", status: data });
      } catch (e) {
        if (!cancelled) setStatusState({ kind: "error", message: String(e) });
      }
    };

    fetchStatus();
    const timer = setInterval(fetchStatus, STATUS_POLL_INTERVAL_MS);

    const handleVisible = () => {
      if (document.visibilityState === "visible") fetchStatus();
    };
    document.addEventListener("visibilitychange", handleVisible);
    window.addEventListener("focus", fetchStatus);

    return () => {
      cancelled = true;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", handleVisible);
      window.removeEventListener("focus", fetchStatus);
    };
  }, [manifest.endpoints.status, bearerToken]);

  const primaryAction = manifest.actions.find((a) => a.primary);
  const secondaryActions = manifest.actions.filter((a) => a !== primaryAction);

  return (
    <div className="flex h-full flex-col gap-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5 transition-colors hover:border-neutral-700">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          {iconUrl && (
            <img
              src={iconUrl}
              alt=""
              className="h-9 w-9 shrink-0 rounded bg-neutral-800 object-cover"
              onError={(e) => ((e.target as HTMLImageElement).style.display = "none")}
            />
          )}
          <div className="min-w-0">
            <h3 className="truncate text-lg font-semibold text-neutral-50">
              {displayName}
            </h3>
            {manifest.category_hint && (
              <p className="text-xs text-neutral-500">
                {labelForCategory(manifest.category_hint)}
              </p>
            )}
          </div>
        </div>
        <ServiceStatusBadge state={statusState} />
      </div>

      {manifest.description && (
        <p className="line-clamp-2 text-sm text-neutral-300">
          {manifest.description}
        </p>
      )}

      {statusState.kind === "ok" && statusState.status.metrics.length > 0 && (
        <MetricsList metrics={statusState.status.metrics} />
      )}

      {statusState.kind === "ok" && statusState.status.badges.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {statusState.status.badges.map((b) => (
            <ManifestBadge key={b.id} badge={b} />
          ))}
        </div>
      )}

      <div className="mt-auto flex items-center justify-end gap-2">
        {secondaryActions.map((action) => (
          <Button
            key={action.id}
            size="sm"
            variant="outline"
            disabled={invokingActionId !== null}
            onClick={() => onAction(action)}
            className="cursor-pointer"
          >
            {invokingActionId === action.id ? "…" : action.label}
          </Button>
        ))}
        {primaryAction && (
          <Button
            size="sm"
            disabled={invokingActionId !== null}
            onClick={() => onAction(primaryAction)}
            className="min-w-[90px] cursor-pointer shadow-sm transition-all hover:shadow-md hover:brightness-110 hover:scale-[1.04] active:scale-[0.96] disabled:cursor-not-allowed"
          >
            {invokingActionId === primaryAction.id ? (
              <span className="inline-flex items-center gap-1.5">
                <Spinner />
                실행 중
              </span>
            ) : (
              primaryAction.label
            )}
          </Button>
        )}
      </div>
    </div>
  );
}

function ServiceStatusBadge({ state }: { state: StatusState }) {
  if (state.kind === "loading") {
    return (
      <Badge className="bg-neutral-800 text-neutral-400">
        <Dot className="bg-neutral-500" />
        확인 중
      </Badge>
    );
  }
  if (state.kind === "error") {
    return (
      <Badge className="bg-yellow-900/30 text-yellow-300" title={state.message}>
        <Dot className="bg-yellow-400" />
        상태 확인 실패
      </Badge>
    );
  }
  if (state.status.online) {
    return (
      <Badge className="bg-emerald-900/40 text-emerald-300">
        <Dot className="bg-emerald-400" />
        온라인
      </Badge>
    );
  }
  return (
    <Badge className="bg-red-900/30 text-red-300">
      <Dot className="bg-red-400" />
      오프라인
    </Badge>
  );
}

function MetricsList({ metrics }: { metrics: Metric[] }) {
  return (
    <ul className="flex flex-col gap-1 text-xs text-neutral-400">
      {metrics.map((m) => (
        <li key={m.id} className="flex items-center justify-between">
          <span>{m.label}</span>
          <span className="font-mono text-neutral-200">{formatMetric(m)}</span>
        </li>
      ))}
    </ul>
  );
}

function formatMetric(metric: Metric): string {
  switch (metric.type) {
    case "players": {
      const v = metric.value as PlayersMetricValue;
      return `${v.online}/${v.max}`;
    }
    case "percentage":
      return `${metric.value}%`;
    case "bytes":
      return formatBytes(Number(metric.value));
    case "timestamp":
      return formatTimestamp(String(metric.value));
    case "number":
      return String(metric.value);
    case "string":
      return String(metric.value);
    default:
      return JSON.stringify(metric.value);
  }
}

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes)) return "-";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = bytes;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

function formatTimestamp(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString();
}

function ManifestBadge({ badge }: { badge: PspBadge }) {
  const colorByLevel = {
    info: "bg-sky-900/40 text-sky-300",
    warning: "bg-yellow-900/30 text-yellow-300",
    error: "bg-red-900/30 text-red-300",
  } as const;
  return (
    <Badge className={colorByLevel[badge.level]} title={badge.id}>
      {badge.label}
    </Badge>
  );
}

function labelForCategory(hint: ServiceManifest["category_hint"]): string {
  switch (hint) {
    case "game":
      return "게임";
    case "media":
      return "미디어";
    case "files":
      return "파일";
    case "communication":
      return "커뮤니케이션";
    case "dev":
      return "개발";
    case "infra":
      return "인프라";
    case "productivity":
      return "생산성";
    case "other":
    default:
      return "기타";
  }
}

function Badge({
  children,
  className = "",
  title,
}: {
  children: React.ReactNode;
  className?: string;
  title?: string;
}) {
  return (
    <span
      title={title}
      className={`flex shrink-0 items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ${className}`}
    >
      {children}
    </span>
  );
}

function Dot({ className = "" }: { className?: string }) {
  return <span className={`h-1.5 w-1.5 rounded-full ${className}`} />;
}

function Spinner() {
  return (
    <span
      className="h-3 w-3 animate-spin rounded-full border-2 border-current border-t-transparent"
      aria-label="실행 중"
    />
  );
}

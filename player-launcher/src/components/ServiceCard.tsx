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

import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Button } from "@/components/ui/button";
import { isPrismInstanceInstalled, isServiceRunning, stopServer } from "@/lib/api";
import type {
  Badge as PspBadge,
  Metric,
  PlayersMetricValue,
  Present,
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
  /** PSP service id — Prism instance dir name 과 동일. running / installed 검사용. */
  serviceId: string;
  /** Authorization header 용 (없으면 미전송). status 와 action 양쪽 사용. */
  bearerToken?: string;
  /** Catalog hint (manifest 보다 우선 표시할 이름/아이콘 — 보통은 manifest 사용). */
  hintName?: string;
  hintIcon?: string;
  /** action 클릭. 부모가 invoke + confirm 재시도 / 결과 처리. */
  onAction: (action: ServiceAction) => void;
  /** 부모가 알리는 invoke 진행 중 action id (해당 버튼 비활성). null = idle. */
  invokingActionId?: string | null;
  /** "앱 제거" 메뉴 — Rust command 호출이 끝나면 resolve. 카드가 후속 state 갱신.
   * 미지정이면 메뉴 자체 안 보임. */
  onRemoveInstance?: () => Promise<void>;
  /** 부모(PspLibrary) 가 1회 detect_prism 후 모든 카드에 전달. third-party 미설치 badge 분기. */
  prismInstalled?: boolean;
}

const STATUS_POLL_INTERVAL_MS = 30_000;

export function ServiceCard({
  manifest,
  serviceId,
  bearerToken,
  hintName,
  hintIcon,
  onAction,
  invokingActionId = null,
  onRemoveInstance,
  prismInstalled,
}: Props) {
  const [statusState, setStatusState] = useState<StatusState>({ kind: "loading" });
  const [menuOpen, setMenuOpen] = useState(false);
  // phase — idle (미실행) / preparing (Prism spawn 완료, 모드팩 다운로드/Mojang 인증)
  // / running (Prism 의 자식으로 minecraft 클라이언트 process 가 등장 = 게임 본체 떠 있음).
  // backend 가 'minecraft:started' event 로 알려줌.
  const [phase, setPhase] = useState<"idle" | "preparing" | "running">("idle");
  const [instanceInstalled, setInstanceInstalled] = useState<boolean | null>(null);
  const [stopping, setStopping] = useState<boolean>(false);

  const running = phase === "running" || phase === "preparing"; // 종료 버튼 활성 등 호환

  const displayName = hintName ?? manifest.name;
  const iconUrl = hintIcon ?? manifest.icon_url;

  // 실행 상태 — 초기 1회 fetch + tauri server:started/stopped 이벤트 listen.
  // 같은 카드의 lifecycle 동안 여러 번 시작/종료될 수 있으니 양쪽 다 처리.
  // 초기 fetch 는 'preparing' 으로 가정 — 이미 띄워진 인스턴스가 어느 단계인지 알 수 없음.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const r = await isServiceRunning(serviceId);
        if (!cancelled && r) setPhase("preparing");
      } catch {
        // ignore — 미지원 환경 (web mode 등)
      }
    })();

    const unlistens: Array<() => void> = [];
    (async () => {
      try {
        const u1 = await listen<{ serverId: string }>("server:started", (e) => {
          if (e.payload.serverId === serviceId) setPhase("preparing");
        });
        const u2 = await listen<{ serverId: string }>("minecraft:started", (e) => {
          if (e.payload.serverId === serviceId) setPhase("running");
        });
        const u3 = await listen<{ serverId: string }>("server:stopped", (e) => {
          if (e.payload.serverId === serviceId) {
            setPhase("idle");
            setStopping(false);
          }
        });
        if (cancelled) {
          u1();
          u2();
          u3();
        } else {
          unlistens.push(u1, u2, u3);
        }
      } catch {
        // ignore
      }
    })();

    return () => {
      cancelled = true;
      unlistens.forEach((u) => u());
    };
  }, [serviceId]);

  // preparing → running 전환은 backend 의 'minecraft:started' event 가 처리.
  // (Prism 자식 process 로 java[w].exe 가 등장하면 emit. 즉 모드팩 다운로드 + Mojang 인증
  // 끝나고 minecraft 본체 클라이언트가 떠있는 시점.)

  // 인스턴스 설치 여부 — 초기 fetch + 실행 시작/종료 시 재검사 (Play 직후 인스턴스 생성됨).
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const v = await isPrismInstanceInstalled(serviceId);
        if (!cancelled) setInstanceInstalled(v);
      } catch {
        if (!cancelled) setInstanceInstalled(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [serviceId, running]);

  const handleStop = async () => {
    setStopping(true);
    try {
      await stopServer(serviceId);
      // server:stopped 이벤트가 곧 도착해서 running=false 로 토글.
    } catch (e) {
      console.warn("[ServiceCard] stop 실패", e);
      setStopping(false);
    }
  };

  // Status: SSE (manifest.endpoints.events) 가 있으면 push 모델 — adapter 가 join/leave
  // 같은 변화를 즉시 보냄. SSE 가 연결될 때 adapter 가 초기 status 도 함께 푸시하므로
  // 별도 첫 fetch 불필요. SSE 가 없거나 끊기면 polling fallback (5분 간격).
  //
  // EventSource 는 표준 API 의 한계로 Authorization 헤더 송신 불가 — adapter 의
  // events_handler 는 `?token=<EVENTS_TOKEN>` query 인증을 지원해서 그 길로 보냄.
  useEffect(() => {
    let cancelled = false;
    let es: EventSource | null = null;
    let pollTimer: ReturnType<typeof setInterval> | null = null;

    const fetchStatusOnce = async () => {
      try {
        const headers: HeadersInit = { Accept: "application/json" };
        if (bearerToken) headers.Authorization = `Bearer ${bearerToken}`;
        const resp = await fetch(manifest.endpoints.status, {
          headers,
          cache: "no-store",
        });
        if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
        const data = (await resp.json()) as StatusResponse;
        if (!cancelled) setStatusState({ kind: "ok", status: data });
      } catch (e) {
        if (!cancelled) setStatusState({ kind: "error", message: String(e) });
      }
    };

    const eventsUrl = manifest.endpoints.events;
    if (eventsUrl) {
      const url = bearerToken
        ? `${eventsUrl}?token=${encodeURIComponent(bearerToken)}`
        : eventsUrl;
      es = new EventSource(url);
      es.addEventListener("status_changed", (e) => {
        if (cancelled) return;
        try {
          const data = JSON.parse((e as MessageEvent).data) as StatusResponse;
          setStatusState({ kind: "ok", status: data });
        } catch (err) {
          console.warn("[ServiceCard] SSE parse error", err);
        }
      });
      es.onerror = () => {
        // EventSource 가 자동 재연결 시도. 첫 연결 실패 시점엔 화면이 비어있을 수
        // 있으므로 1회 polling 으로 fallback. 끊겼다 다시 붙으면 SSE 재연결이 push 하는
        // status_changed 가 갱신.
        if (!cancelled && statusState.kind === "loading") {
          void fetchStatusOnce();
        }
      };
    } else {
      // events endpoint 가 없는 service — polling 모델
      void fetchStatusOnce();
      pollTimer = setInterval(fetchStatusOnce, STATUS_POLL_INTERVAL_MS);
    }

    // SSE 든 polling 든, focus / visibility 변화 시 한 번 더 fetch (네트워크 sleep 회복).
    const refreshOnFocus = () => void fetchStatusOnce();
    const handleVisible = () => {
      if (document.visibilityState === "visible") refreshOnFocus();
    };
    document.addEventListener("visibilitychange", handleVisible);
    window.addEventListener("focus", refreshOnFocus);

    return () => {
      cancelled = true;
      if (es) es.close();
      if (pollTimer) clearInterval(pollTimer);
      document.removeEventListener("visibilitychange", handleVisible);
      window.removeEventListener("focus", refreshOnFocus);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [manifest.endpoints.events, manifest.endpoints.status, bearerToken]);

  const primaryAction = manifest.actions.find((a) => a.primary);
  const secondaryActions = manifest.actions.filter((a) => a !== primaryAction);

  // 카드의 border 색 — phase 별로 분기. preparing 은 amber, running 은 emerald.
  const cardBorder =
    phase === "running"
      ? "border-emerald-500/60 ring-1 ring-emerald-500/20"
      : phase === "preparing"
        ? "border-amber-500/60 ring-1 ring-amber-500/20"
        : "border-neutral-800 hover:border-neutral-700";

  // 추가 상태 badge — 정상 상태에선 표시 안 함 (시각 노이즈 최소화).
  const showPrismMissingBadge =
    prismInstalled === false &&
    manifest.actions.some((a) => a.kind === "native_third_party_app");
  const showNotInstalledBadge =
    instanceInstalled === false && !showPrismMissingBadge;

  return (
    <div
      className={`flex h-full flex-col gap-3 rounded-lg border bg-neutral-900/60 p-5 transition-colors ${cardBorder}`}
    >
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
        <div className="flex flex-col items-end gap-1">
          <div className="flex items-center gap-1.5">
            <ServiceStatusBadge state={statusState} />
            {onRemoveInstance && instanceInstalled !== false && (
              <CardMenu
                open={menuOpen}
                onOpenChange={setMenuOpen}
                onRemoveInstance={async () => {
                  try {
                    await onRemoveInstance();
                    // Rust 측 폴더 삭제 성공 → 즉시 state 반영 (메뉴 숨김 + "미설치" badge).
                    setInstanceInstalled(false);
                  } catch {
                    // 부모가 toast 처리. state 변경 없이 그대로.
                  }
                }}
                displayName={displayName}
              />
            )}
          </div>
          {phase === "preparing" && (
            <Badge className="bg-amber-900/50 text-amber-200" title="모드팩 다운로드 / Mojang 인증 / world load 중">
              <Spinner />
              준비 중
            </Badge>
          )}
          {phase === "running" && (
            <Badge className="bg-emerald-900/50 text-emerald-200">
              <Dot className="bg-emerald-400 animate-pulse" />
              실행 중
            </Badge>
          )}
          {showPrismMissingBadge && (
            <Badge className="bg-yellow-900/40 text-yellow-200" title="이 서비스는 PrismLauncher 가 필요합니다">
              <Dot className="bg-yellow-400" />
              PrismLauncher 필요
            </Badge>
          )}
          {showNotInstalledBadge && (
            <Badge className="bg-neutral-800 text-neutral-300" title="첫 실행 시 자동 설치">
              <Dot className="bg-neutral-500" />
              미설치
            </Badge>
          )}
        </div>
      </div>

      {manifest.description && (
        <p className="line-clamp-2 text-sm text-neutral-300">
          {manifest.description}
        </p>
      )}

      {statusState.kind === "ok" && statusState.status.metrics.length > 0 && (
        <MetricsList metrics={statusState.status.metrics} />
      )}

      {statusState.kind === "ok" &&
        (statusState.status.present?.length ?? 0) > 0 && (
          <PresentChips present={statusState.status.present!} />
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
            disabled={invokingActionId !== null || running}
            onClick={() => onAction(action)}
            className="cursor-pointer"
          >
            {invokingActionId === action.id ? "…" : action.label}
          </Button>
        ))}
        {running ? (
          // 실행 중 — primary action 자리에 종료 버튼.
          <Button
            size="sm"
            variant="outline"
            disabled={stopping}
            onClick={() => void handleStop()}
            className="min-w-[90px] cursor-pointer border-red-700/60 text-red-200 hover:bg-red-950/40 disabled:cursor-not-allowed"
          >
            {stopping ? (
              <span className="inline-flex items-center gap-1.5">
                <Spinner />
                종료 중
              </span>
            ) : (
              "종료"
            )}
          </Button>
        ) : (
          primaryAction && (
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
          )
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

// 현재 접속자 — presence(모임 레이어)의 service-anchored 표시.
// status.present (각 service 가 자기 status 로 보고) 를 client 가 칩으로 렌더만 한다.
// PengPort 는 presence 를 호스팅하지 않음. 표시 전용 (합류는 Play 버튼). id/label 은
// 어댑터발 untrusted — React 기본 escape 에 의존, 표시 외 사용 금지.
function PresentChips({ present }: { present: Present[] }) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {present.map((p) => (
        <span
          key={p.id}
          className="flex shrink-0 items-center gap-1.5 rounded-full bg-neutral-800 px-2.5 py-0.5 text-xs font-medium text-neutral-200"
        >
          <Dot className="bg-emerald-400" />
          {p.label ?? p.id}
        </span>
      ))}
    </div>
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

// ============================================================
// 카드 우상단 [⋮] 메뉴 — 현재는 "Prism 인스턴스 삭제" 만.
// 자체 popover (base-ui Menu 미사용) — 항목 1개라 단순 구현.
// ============================================================
function CardMenu({
  open,
  onOpenChange,
  onRemoveInstance,
  displayName,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRemoveInstance: () => Promise<void>;
  displayName: string;
}) {
  const ref = useRef<HTMLDivElement>(null);

  // 바깥 클릭 / ESC 닫기
  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        onOpenChange(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onOpenChange(false);
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onOpenChange]);

  const handleRemove = () => {
    onOpenChange(false);
    const ok = confirm(
      `${displayName} 을(를) 제거할까요?\n\n` +
        `게임 데이터 (saves, mods, config 등) 가 모두 사라집니다.\n` +
        `다시 실행하면 처음부터 자동 설치됩니다 (saves 는 복구 불가).`,
    );
    if (ok) void onRemoveInstance();
  };

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        aria-label="더보기"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => onOpenChange(!open)}
        className="flex h-7 w-7 cursor-pointer items-center justify-center rounded text-neutral-500 transition-colors hover:bg-neutral-800/60 hover:text-neutral-200"
      >
        <svg
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="currentColor"
          aria-hidden
        >
          <circle cx="5" cy="12" r="1.6" />
          <circle cx="12" cy="12" r="1.6" />
          <circle cx="19" cy="12" r="1.6" />
        </svg>
      </button>
      {open && (
        <div
          role="menu"
          className="absolute right-0 top-full z-10 mt-1 w-56 overflow-hidden rounded-md border border-neutral-700 bg-neutral-900 py-1 shadow-lg"
        >
          <button
            type="button"
            role="menuitem"
            onClick={handleRemove}
            className="flex w-full cursor-pointer items-center px-3 py-1.5 text-left text-xs text-red-300 transition-colors hover:bg-red-950/50"
          >
            앱 제거
          </button>
        </div>
      )}
    </div>
  );
}

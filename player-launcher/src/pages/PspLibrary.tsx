// PSP 라이브러리 (메인 페이지).
//
// 흐름:
// 1. 저장된 instance URL/token 이 없으면 OOBE — `<InstanceSetup>` 폼 표시
// 2. 있으면 instance metadata + catalog fetch → 각 service 의 manifest fetch
// 3. ServiceCard 렌더링 (status polling, action 버튼)
// 4. NeedsConfirm 받으면 ConsentDialog → 동의 후 invoke 재시도
//
// 인스턴스 / 토큰 영속화: localStorage (간단). OS keychain 통합은 추후 트랙 14 잔여.

import { useCallback, useEffect, useState } from "react";
import { ConsentDialog, type ConsentRequest } from "@/components/ConsentDialog";
import { ServiceCard } from "@/components/ServiceCard";
import { Button } from "@/components/ui/button";
import {
  catalogCache,
  instanceCache,
  manifestCache,
  pspInvokeAction,
  pspLoadCatalog,
  pspLoadInstance,
  pspLoadManifest,
  pspTrust,
  type ActionOutcome,
  type InstanceMetadata,
  type ServiceAction,
  type ServiceEntry,
  type ServiceManifest,
  type ServicesCatalog,
} from "@/lib/psp";

// ====== 영속화 ======

const LS_INSTANCE_URL = "pengport.instance_url";
const LS_INSTANCE_TOKEN = "pengport.instance_token";

function loadInstanceUrl(): string | null {
  return localStorage.getItem(LS_INSTANCE_URL);
}

function loadInstanceToken(): string | null {
  return localStorage.getItem(LS_INSTANCE_TOKEN);
}

function saveInstance(url: string, token: string | null) {
  localStorage.setItem(LS_INSTANCE_URL, url);
  if (token) {
    localStorage.setItem(LS_INSTANCE_TOKEN, token);
  } else {
    localStorage.removeItem(LS_INSTANCE_TOKEN);
  }
}

function clearInstance() {
  localStorage.removeItem(LS_INSTANCE_URL);
  localStorage.removeItem(LS_INSTANCE_TOKEN);
}

// ====== 컴포넌트 상태 ======

interface ServiceState {
  entry: ServiceEntry;
  manifest: ServiceManifest;
}

type LoadState =
  | { kind: "needs_setup" }
  | { kind: "loading" }
  | {
      kind: "ready";
      instance: InstanceMetadata;
      services: ServiceState[];
      bearerToken: string | null;
    }
  | { kind: "error"; message: string; instanceUrl: string };

interface PendingAction {
  service: ServiceState;
  action: ServiceAction;
}

interface ToastState {
  kind: "info" | "error";
  message: string;
}

export default function PspLibrary() {
  const [state, setState] = useState<LoadState>({ kind: "needs_setup" });
  const [invokingActionId, setInvokingActionId] = useState<string | null>(null);
  const [pendingConfirm, setPendingConfirm] = useState<{
    request: ConsentRequest;
    pending: PendingAction;
  } | null>(null);
  const [confirmProcessing, setConfirmProcessing] = useState(false);
  const [toast, setToast] = useState<ToastState | null>(null);

  const loadFromInstance = useCallback(
    async (instanceUrl: string, bearerToken: string | null) => {
      setState({ kind: "loading" });
      try {
        const instance =
          instanceCache.get(instanceUrl) ??
          (await pspLoadInstance(instanceUrl));
        instanceCache.set(instanceUrl, instance);

        const catalogUrl = instance.endpoints.catalog;
        const catalog: ServicesCatalog =
          catalogCache.get(catalogUrl) ??
          (await pspLoadCatalog(catalogUrl, bearerToken ?? undefined));
        catalogCache.set(catalogUrl, catalog);

        const enabled = catalog.services.filter((s) => s.enabled);
        const manifests: ServiceState[] = [];
        for (const entry of enabled) {
          try {
            const manifest =
              manifestCache.get(entry.url) ??
              (await pspLoadManifest(entry.url, bearerToken ?? undefined));
            manifestCache.set(entry.url, manifest);
            manifests.push({ entry, manifest });
          } catch (e) {
            // 한 service 실패가 전체를 막지 않도록 — 로그 + skip
            console.warn(`manifest fetch 실패 (${entry.id}):`, e);
          }
        }
        setState({
          kind: "ready",
          instance,
          services: manifests,
          bearerToken,
        });
      } catch (e) {
        setState({ kind: "error", message: String(e), instanceUrl });
      }
    },
    [],
  );

  // 마운트 시 저장된 instance 로드 시도.
  useEffect(() => {
    const url = loadInstanceUrl();
    if (!url) {
      setState({ kind: "needs_setup" });
      return;
    }
    const token = loadInstanceToken();
    void loadFromInstance(url, token);
  }, [loadFromInstance]);

  const handleSetupSubmit = useCallback(
    async (url: string, token: string) => {
      saveInstance(url, token || null);
      await loadFromInstance(url, token || null);
    },
    [loadFromInstance],
  );

  const handleClearInstance = useCallback(() => {
    clearInstance();
    instanceCache.clear();
    catalogCache.clear();
    manifestCache.clear();
    setState({ kind: "needs_setup" });
  }, []);

  // ====== Action invoke ======

  const invoke = useCallback(
    async (
      service: ServiceState,
      action: ServiceAction,
    ): Promise<ActionOutcome> => {
      return pspInvokeAction({
        kind: action.kind,
        args: action.args,
        manifestOrigin: service.entry.url,
        externalUrls: service.manifest.permissions.external_urls,
        instanceId: service.entry.id,
      });
    },
    [],
  );

  const handleOutcome = useCallback(
    (action: ServiceAction, service: ServiceState, outcome: ActionOutcome) => {
      switch (outcome.kind) {
        case "done":
          setToast({ kind: "info", message: `${action.label} 완료` });
          return;
        case "submitted":
          setToast({
            kind: outcome.status >= 400 ? "error" : "info",
            message: `${action.label}: HTTP ${outcome.status}`,
          });
          return;
        case "launched":
          setToast({
            kind: "info",
            message: `${action.label} 실행: ${outcome.instance_id}`,
          });
          return;
        case "needs_confirm":
          setPendingConfirm({
            request: outcome,
            pending: { service, action },
          });
          return;
      }
    },
    [],
  );

  const handleAction = useCallback(
    (service: ServiceState) => async (action: ServiceAction) => {
      setInvokingActionId(action.id);
      try {
        const outcome = await invoke(service, action);
        handleOutcome(action, service, outcome);
      } catch (e) {
        setToast({ kind: "error", message: String(e) });
      } finally {
        setInvokingActionId(null);
      }
    },
    [invoke, handleOutcome],
  );

  const handleAllowConfirm = useCallback(
    async (req: ConsentRequest) => {
      if (!pendingConfirm) return;
      setConfirmProcessing(true);
      try {
        await pspTrust({
          trustKind: req.trust_kind,
          subjectId: req.subject_id,
          display: req.display,
          metadata: req.details,
        });
        const { service, action } = pendingConfirm.pending;
        const retry = await invoke(service, action);
        if (retry.kind === "needs_confirm") {
          setPendingConfirm({ request: retry, pending: pendingConfirm.pending });
        } else {
          setPendingConfirm(null);
          handleOutcome(action, service, retry);
        }
      } catch (e) {
        setToast({ kind: "error", message: `허용 실패: ${e}` });
        setPendingConfirm(null);
      } finally {
        setConfirmProcessing(false);
      }
    },
    [pendingConfirm, invoke, handleOutcome],
  );

  const handleDenyConfirm = useCallback(() => {
    setPendingConfirm(null);
  }, []);

  // ====== 렌더 ======

  return (
    <div className="p-8">
      {state.kind === "needs_setup" && (
        <InstanceSetup onSubmit={handleSetupSubmit} />
      )}

      {state.kind === "loading" && (
        <p className="text-sm text-neutral-400">인스턴스 정보 불러오는 중...</p>
      )}

      {state.kind === "error" && (
        <div className="space-y-3 rounded-md border border-red-900/50 bg-red-900/20 p-4 text-sm text-red-200">
          <p className="font-medium">인스턴스 연결 실패</p>
          <p className="text-xs text-red-300/80 break-all">{state.message}</p>
          <p className="text-xs text-red-300/70 break-all">
            URL: {state.instanceUrl}
          </p>
          <div className="flex gap-2">
            <Button
              size="sm"
              variant="outline"
              onClick={() =>
                loadFromInstance(state.instanceUrl, loadInstanceToken())
              }
            >
              다시 시도
            </Button>
            <Button size="sm" variant="outline" onClick={handleClearInstance}>
              인스턴스 변경
            </Button>
          </div>
        </div>
      )}

      {state.kind === "ready" && (
        <>
          <header className="mb-6 flex items-baseline justify-between">
            <div>
              <h2 className="text-2xl font-semibold">{state.instance.name}</h2>
              <p className="mt-1 text-xs text-neutral-500">
                {state.instance.operator.name}
                {state.instance.description && ` · ${state.instance.description}`}
              </p>
            </div>
            <Button size="sm" variant="outline" onClick={handleClearInstance}>
              인스턴스 변경
            </Button>
          </header>

          {state.services.length === 0 ? (
            <p className="text-sm text-neutral-400">활성 서비스가 없습니다.</p>
          ) : (
            <ul className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
              {state.services.map(({ entry, manifest }) => (
                <li key={entry.id}>
                  <ServiceCard
                    manifest={manifest}
                    bearerToken={state.bearerToken ?? undefined}
                    hintName={entry.hint?.name}
                    hintIcon={entry.hint?.icon}
                    onAction={handleAction({ entry, manifest })}
                    invokingActionId={invokingActionId}
                  />
                </li>
              ))}
            </ul>
          )}
        </>
      )}

      <ConsentDialog
        request={pendingConfirm?.request ?? null}
        onAllow={handleAllowConfirm}
        onDeny={handleDenyConfirm}
        processing={confirmProcessing}
      />

      {toast && (
        <div
          className={`fixed bottom-6 right-6 max-w-sm cursor-pointer rounded-lg border px-4 py-3 text-sm shadow-lg ${
            toast.kind === "error"
              ? "border-red-900/60 bg-red-950/80 text-red-200"
              : "border-emerald-900/60 bg-emerald-950/80 text-emerald-200"
          }`}
          onClick={() => setToast(null)}
          role="status"
        >
          {toast.message}
        </div>
      )}
    </div>
  );
}

// ====== OOBE: 인스턴스 추가 form ======

function InstanceSetup({
  onSubmit,
}: {
  onSubmit: (url: string, token: string) => Promise<void>;
}) {
  const [url, setUrl] = useState("");
  const [token, setToken] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handle = async () => {
    setSubmitting(true);
    setError(null);
    try {
      await onSubmit(url.trim(), token.trim());
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="mx-auto max-w-lg space-y-4 rounded-lg border border-sky-900/50 bg-sky-900/15 p-6">
      <h2 className="text-lg font-semibold text-sky-100">PengPort 사용 시작</h2>
      <p className="text-sm text-sky-200/90">
        연결할 PengPort 인스턴스의 URL 을 입력하세요. 운영자가 알려준 도메인입니다
        (예: <code>https://pengdoll.duckdns.org</code>).
      </p>

      <div className="space-y-2">
        <label className="text-xs text-sky-200/80">인스턴스 URL</label>
        <input
          type="text"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="https://pengdoll.duckdns.org"
          className="w-full rounded bg-neutral-950 px-3 py-2 font-mono text-sm text-neutral-100 outline-none ring-1 ring-neutral-800 focus:ring-sky-700"
          autoFocus
          spellCheck={false}
        />
      </div>

      <div className="space-y-2">
        <label className="text-xs text-sky-200/80">
          토큰 (선택 — auth.type=token 인스턴스만)
        </label>
        <input
          type="text"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder="(선택사항) 운영자에게 받은 토큰"
          className="w-full rounded bg-neutral-950 px-3 py-2 font-mono text-xs text-neutral-100 outline-none ring-1 ring-neutral-800 focus:ring-sky-700"
          spellCheck={false}
        />
      </div>

      {error && <p className="text-xs text-red-300">실패: {error}</p>}

      <Button
        size="sm"
        onClick={handle}
        disabled={submitting || !url.trim()}
      >
        {submitting ? "연결 중..." : "연결"}
      </Button>
    </div>
  );
}

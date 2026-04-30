// PSP 라이브러리 (메인 페이지) — multi-instance 모델.
//
// 흐름:
// 1. 등록된 instance 가 없으면 OOBE — `<InstanceSetup>` 폼 표시
// 2. active instance 가 있으면 metadata + catalog fetch → 각 service 의 manifest fetch
// 3. ServiceCard 렌더링 (status polling, action 버튼)
// 4. NeedsConfirm 받으면 ConsentDialog → 동의 후 invoke 재시도
//
// 영속화 (lib/instances.ts + lib/secrets.ts):
// - instance list / active id : localStorage (시크릿 아님)
// - 각 instance 의 bearer token: OS keychain (instance.id 별 격리)
//
// active instance 전환은 사이드바 (App.tsx) 가 처리. 이 컴포넌트는 useInstances()
// 의 active 가 바뀌면 자동으로 다시 catalog 로드.

import { useCallback, useEffect, useState } from "react";
import { ConsentDialog, type ConsentRequest } from "@/components/ConsentDialog";
import {
  ThirdPartyInstallDialog,
  type InstallRequest,
} from "@/components/ThirdPartyInstallDialog";
import { ServiceCard } from "@/components/ServiceCard";
import { Button } from "@/components/ui/button";
import { removePrismInstance } from "@/lib/api";
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
import { useInstances } from "@/lib/instances-context";
import { instanceToken } from "@/lib/secrets";

// ====== 컴포넌트 상태 ======

interface ServiceState {
  entry: ServiceEntry;
  manifest: ServiceManifest;
}

type LoadState =
  | { kind: "needs_setup" }
  | { kind: "loading"; instanceId: string }
  | {
      kind: "ready";
      instanceId: string;
      instance: InstanceMetadata;
      services: ServiceState[];
      bearerToken: string | null;
    }
  | { kind: "error"; instanceId: string; instanceUrl: string; message: string };

interface PendingAction {
  service: ServiceState;
  action: ServiceAction;
}

interface ToastState {
  kind: "info" | "error";
  message: string;
}

export default function PspLibrary() {
  const { active, remove, updateName } = useInstances();
  const [state, setState] = useState<LoadState>({ kind: "needs_setup" });
  const [invokingActionId, setInvokingActionId] = useState<string | null>(null);
  const [pendingConfirm, setPendingConfirm] = useState<{
    request: ConsentRequest;
    pending: PendingAction;
  } | null>(null);
  const [confirmProcessing, setConfirmProcessing] = useState(false);
  const [pendingInstall, setPendingInstall] = useState<{
    request: InstallRequest;
    pending: PendingAction;
  } | null>(null);
  const [toast, setToast] = useState<ToastState | null>(null);

  const loadFromInstance = useCallback(
    async (instanceId: string, instanceUrl: string) => {
      setState({ kind: "loading", instanceId });
      try {
        const bearerToken = await instanceToken.load(instanceId);

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
          instanceId,
          instance,
          services: manifests,
          bearerToken,
        });
      } catch (e) {
        setState({
          kind: "error",
          instanceId,
          instanceUrl,
          message: String(e),
        });
      }
    },
    [],
  );

  // active instance 가 바뀔 때마다 reload.
  // 의존성은 id/url primitive 만 (active object 자체는 매 render 새 reference 라
  // useEffect 무한 루프 유발 — instance list 변경 → active 재계산 → 재 fetch → ...).
  const activeIdDep = active?.id ?? null;
  const activeUrlDep = active?.url ?? null;
  useEffect(() => {
    if (!activeIdDep || !activeUrlDep) {
      setState({ kind: "needs_setup" });
      return;
    }
    void loadFromInstance(activeIdDep, activeUrlDep);
  }, [activeIdDep, activeUrlDep, loadFromInstance]);

  // catalog 로드 성공 시 instance metadata 의 name 을 사이드바 표시용으로 자동 채움.
  // useEffect 분리: loadFromInstance 안에서 직접 호출하면 updateName → instances state 변경
  // → active 재생성 → useEffect 재실행 의 무한 루프. ready state 에서만 idempotent 호출.
  useEffect(() => {
    if (state.kind !== "ready") return;
    const current = active;
    if (current && current.id === state.instanceId) {
      const newName = state.instance.name;
      if (newName && current.name !== newName) {
        updateName(current.id, newName);
      }
    }
  }, [state, active, updateName]);

  const handleRemoveActive = useCallback(() => {
    if (!active) return;
    void (async () => {
      // active instance 만 제거. context 가 다른 instance 또는 null 로 active 변경.
      // catalog/manifest cache 도 정리 (URL 기반이라 다른 instance 영향 없지만 깔끔하게).
      instanceCache.clear();
      catalogCache.clear();
      manifestCache.clear();
      await remove(active.id);
    })();
  }, [active, remove]);

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
        case "third_party_missing":
          setPendingInstall({
            request: {
              app_id: outcome.app_id,
              install_hint: outcome.install_hint,
            },
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

  // ThirdPartyInstallDialog: 사용자가 자동 설치 완료한 후 원래 action 재시도.
  // 같은 흐름 안에서 ThirdPartyMissing 이 또 떨어지면 (드물지만 가능) dialog 재오픈.
  const handleThirdPartyInstalled = useCallback(
    async (_req: InstallRequest) => {
      if (!pendingInstall) return;
      const { service, action } = pendingInstall.pending;
      setPendingInstall(null);
      try {
        const retry = await invoke(service, action);
        if (retry.kind === "third_party_missing") {
          setPendingInstall({
            request: {
              app_id: retry.app_id,
              install_hint: retry.install_hint,
            },
            pending: { service, action },
          });
        } else {
          handleOutcome(action, service, retry);
        }
      } catch (e) {
        setToast({ kind: "error", message: String(e) });
      }
    },
    [pendingInstall, invoke, handleOutcome],
  );

  const handleThirdPartyCancel = useCallback(() => {
    setPendingInstall(null);
  }, []);

  // ServiceCard 의 "Prism 인스턴스 삭제" 메뉴 — confirm 없이 호출 (카드가 자체 confirm 처리).
  const handleRemoveServiceInstance = useCallback(
    async (service: ServiceState) => {
      try {
        await removePrismInstance(service.entry.id);
        setToast({
          kind: "info",
          message: `${service.manifest.name} 의 Prism 인스턴스 폴더 삭제 완료`,
        });
      } catch (e) {
        setToast({ kind: "error", message: `삭제 실패: ${e}` });
      }
    },
    [],
  );

  // ====== 렌더 ======

  return (
    <div className="p-8">
      {state.kind === "needs_setup" && <InstanceSetup />}

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
              onClick={() => void loadFromInstance(state.instanceId, state.instanceUrl)}
            >
              다시 시도
            </Button>
            <Button size="sm" variant="outline" onClick={handleRemoveActive}>
              인스턴스 제거
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
                {state.instance.description &&
                  ` · ${state.instance.description}`}
              </p>
            </div>
            <Button size="sm" variant="outline" onClick={handleRemoveActive}>
              인스턴스 제거
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
                    onRemoveInstance={() =>
                      void handleRemoveServiceInstance({ entry, manifest })
                    }
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

      <ThirdPartyInstallDialog
        request={pendingInstall?.request ?? null}
        onInstalled={handleThirdPartyInstalled}
        onCancel={handleThirdPartyCancel}
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
//
// instance 가 하나도 없을 때 (또는 사용자가 사이드바의 "인스턴스 추가" 클릭 시) 표시.
// add → keyring 에 token 저장 → context 가 active 로 설정 → useEffect 가 자동 reload.

export function InstanceSetup() {
  const { add } = useInstances();
  const [url, setUrl] = useState("");
  const [token, setToken] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handle = async () => {
    setSubmitting(true);
    setError(null);
    try {
      const trimmedUrl = url.trim();
      const trimmedToken = token.trim();
      if (!trimmedUrl) {
        setError("URL 이 비어있습니다.");
        setSubmitting(false);
        return;
      }
      const entry = add({ url: trimmedUrl });
      if (trimmedToken) {
        await instanceToken.save(entry.id, trimmedToken);
      }
      // context 가 active 변경을 감지 → PspLibrary 가 자동 catalog 로드
    } catch (e) {
      setError(String(e));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="mx-auto max-w-lg space-y-4 rounded-lg border border-sky-900/50 bg-sky-900/15 p-6">
      <h2 className="text-lg font-semibold text-sky-100">인스턴스 추가</h2>
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

      <Button size="sm" onClick={handle} disabled={submitting || !url.trim()}>
        {submitting ? "추가 중..." : "추가"}
      </Button>
    </div>
  );
}

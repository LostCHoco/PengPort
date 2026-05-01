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
import { detectPrism, removePrismInstance } from "@/lib/api";
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
import { isSameOrigin } from "@/lib/url";
import { getMode } from "@/lib/mode";

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
  const { active, updateName, reloadKey } = useInstances();
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
  const [prismInstalled, setPrismInstalled] = useState<boolean | null>(null);

  // PrismLauncher 설치 여부 — 1회 fetch 후 모든 ServiceCard 에 전달. 설치 dialog 가 끝나면
  // re-detect 해서 badge 갱신.
  //
  // ephemeral 모드 (1회용 PC) 에선 system Prism 을 사용하면 안 된다 — 다른 user 의 Microsoft
  // 계정 데이터에 PengPort 가 기여하면 PC 떠난 후 그 사용자의 계정으로 minecraft 접속 가능
  // (token 잔재). 그래서 ephemeral 시 bundled (PengPort 가 다운받은 격리된 prism) 만 인정 →
  // bundled 없으면 ThirdPartyInstallDialog 가 download 트리거 → ephemeral 종료 시 같이 wipe.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const loc = await detectPrism();
        if (!cancelled) {
          const ephemeral = getMode() === "ephemeral";
          const acceptable = loc !== null && (!ephemeral || loc.source === "bundled");
          setPrismInstalled(acceptable);
        }
      } catch {
        if (!cancelled) setPrismInstalled(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pendingInstall]);

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

        // 보안: catalog URL 이 instance origin 과 같은 origin 인지 검증.
        // 다르면 token 이 cross-origin 으로 누출됨 (e.g. attacker instance 가
        // endpoints.catalog 를 다른 도메인으로 가리키게 해서 그 도메인이 token 받음).
        // PSP 의 same-origin policy 강제 — catalog 는 instance 의 일부여야 한다.
        if (!isSameOrigin(instanceUrl, catalogUrl)) {
          throw new Error(
            `보안 차단: catalog URL (${catalogUrl}) 이 instance origin (${instanceUrl}) 과 다른 origin 입니다. ` +
              `토큰 누출 위험으로 fetch 거부. 운영자에게 instance metadata 의 endpoints.catalog 확인 요청 필요.`,
          );
        }

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
  // reloadKey 는 같은 active 안에서도 강제 재fetch 가 필요한 시나리오 (토큰 갱신 등) 의 trigger.
  const activeIdDep = active?.id ?? null;
  const activeUrlDep = active?.url ?? null;
  useEffect(() => {
    if (!activeIdDep || !activeUrlDep) {
      setState({ kind: "needs_setup" });
      return;
    }
    void loadFromInstance(activeIdDep, activeUrlDep);
  }, [activeIdDep, activeUrlDep, reloadKey, loadFromInstance]);

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
            message: `${service.manifest.name} 실행 시작`,
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

  // ServiceCard 의 "앱 제거" 메뉴 — confirm 없이 호출 (카드가 자체 confirm 처리).
  // 성공 시 ServiceCard 가 자기 상태 (instanceInstalled=false) 갱신하도록 Promise 반환.
  const handleRemoveServiceInstance = useCallback(
    async (service: ServiceState): Promise<void> => {
      try {
        await removePrismInstance(service.entry.id);
        setToast({
          kind: "info",
          message: `${service.manifest.name} 제거 완료`,
        });
      } catch (e) {
        setToast({ kind: "error", message: `제거 실패: ${e}` });
        throw e;
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
          </div>
          <p className="text-xs text-red-300/60">
            계속 실패하면 설정 → 인스턴스 관리에서 제거 후 다시 추가하세요.
          </p>
        </div>
      )}

      {state.kind === "ready" && (
        <>
          <header className="mb-6">
            <h2 className="text-2xl font-semibold">{state.instance.name}</h2>
          </header>

          {state.services.length === 0 ? (
            <p className="text-sm text-neutral-400">활성 서비스가 없습니다.</p>
          ) : (
            <ul className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
              {state.services.map(({ entry, manifest }) => (
                <li key={entry.id}>
                  <ServiceCard
                    manifest={manifest}
                    serviceId={entry.id}
                    bearerToken={state.bearerToken ?? undefined}
                    hintName={entry.hint?.name}
                    hintIcon={entry.hint?.icon}
                    onAction={handleAction({ entry, manifest })}
                    invokingActionId={invokingActionId}
                    prismInstalled={prismInstalled ?? undefined}
                    onRemoveInstance={() =>
                      handleRemoveServiceInstance({ entry, manifest })
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
        연결할 PengPort 인스턴스의 URL 을 입력하세요. 운영자가 알려준 도메인입니다.
      </p>

      <div className="space-y-2">
        <label className="text-xs text-sky-200/80">인스턴스 URL</label>
        <input
          type="text"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="https://..."
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

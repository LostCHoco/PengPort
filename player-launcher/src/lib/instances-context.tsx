// Multi-instance 상태 React context.
//
// PspLibrary (active instance 의 catalog 표시) 와 App.tsx 사이드바 (instance list)
// 양쪽이 같은 instance state 를 공유. lib/instances.ts 의 localStorage 를 source of truth
// 로 두고 React state 는 mirror.
//
// mount 시 옛 schema (단일 instance_url + keyring 'instance_token') 자동 마이그레이션.

import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useEffect,
  useState,
} from "react";
import {
  addInstance as addInstanceLs,
  getActiveInstanceId,
  type InstanceEntry,
  loadInstances,
  migrateLegacy,
  removeInstance as removeInstanceLs,
  setActiveInstanceId as persistActiveId,
  updateInstanceName as updateNameLs,
} from "./instances";
import { catalogCache, instanceCache, manifestCache } from "./psp";

// 인스턴스 추가/제거 시 PSP 메모리 cache 를 모두 비운다.
// 그렇지 않으면 같은 URL 의 instance 를 [제거] 한 직후 다시 [추가] 했을 때, 5분 TTL 안의
// cached catalog/manifest 가 그대로 hit 되어 새 토큰 없이도 service 가 표시되는 false-positive
// 가 발생한다 (서버는 401 반환하지만 fetch 자체가 안 일어남).
function invalidatePspCaches() {
  instanceCache.clear();
  catalogCache.clear();
  manifestCache.clear();
}

interface InstancesContextValue {
  instances: InstanceEntry[];
  activeId: string | null;
  active: InstanceEntry | null;
  add: (input: { url: string; name?: string }) => InstanceEntry;
  remove: (id: string) => Promise<void>;
  setActive: (id: string | null) => void;
  updateName: (id: string, name: string | undefined) => void;
  /** localStorage 를 외부에서 직접 비운 후 React state 동기화 (예: 데이터 초기화). */
  refresh: () => void;
}

const InstancesContext = createContext<InstancesContextValue | null>(null);

export function InstancesProvider({ children }: { children: ReactNode }) {
  const [instances, setInstances] = useState<InstanceEntry[]>(() =>
    loadInstances(),
  );
  const [activeId, setActiveIdState] = useState<string | null>(() =>
    getActiveInstanceId(),
  );

  // mount 시 한 번만 옛 schema 마이그레이션. 변환됐으면 state 갱신.
  useEffect(() => {
    void (async () => {
      const migrated = await migrateLegacy();
      if (migrated) {
        setInstances(loadInstances());
        setActiveIdState(getActiveInstanceId());
      }
    })();
  }, []);

  const refresh = useCallback(() => {
    setInstances(loadInstances());
    setActiveIdState(getActiveInstanceId());
  }, []);

  const add = useCallback(
    (input: { url: string; name?: string }) => {
      // 옛 cached catalog/manifest 가 token 없이 reuse 되는 false-positive 방지.
      invalidatePspCaches();
      const entry = addInstanceLs(input);
      refresh();
      return entry;
    },
    [refresh],
  );

  const remove = useCallback(
    async (id: string) => {
      await removeInstanceLs(id);
      // 같은 URL 로 다시 추가했을 때 옛 cache 가 hit 되어 token 없이 작동하는 버그 방지.
      invalidatePspCaches();
      refresh();
    },
    [refresh],
  );

  const setActive = useCallback((id: string | null) => {
    persistActiveId(id);
    setActiveIdState(id);
  }, []);

  const updateName = useCallback(
    (id: string, name: string | undefined) => {
      updateNameLs(id, name);
      refresh();
    },
    [refresh],
  );

  const active = activeId
    ? instances.find((i) => i.id === activeId) ?? null
    : null;

  return (
    <InstancesContext.Provider
      value={{ instances, activeId, active, add, remove, setActive, updateName, refresh }}
    >
      {children}
    </InstancesContext.Provider>
  );
}

export function useInstances(): InstancesContextValue {
  const ctx = useContext(InstancesContext);
  if (!ctx)
    throw new Error("useInstances must be used inside <InstancesProvider>");
  return ctx;
}

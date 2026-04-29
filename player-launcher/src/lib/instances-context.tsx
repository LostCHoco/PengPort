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

interface InstancesContextValue {
  instances: InstanceEntry[];
  activeId: string | null;
  active: InstanceEntry | null;
  add: (input: { url: string; name?: string }) => InstanceEntry;
  remove: (id: string) => Promise<void>;
  setActive: (id: string | null) => void;
  updateName: (id: string, name: string | undefined) => void;
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
      const entry = addInstanceLs(input);
      refresh();
      return entry;
    },
    [refresh],
  );

  const remove = useCallback(
    async (id: string) => {
      await removeInstanceLs(id);
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
      value={{ instances, activeId, active, add, remove, setActive, updateName }}
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

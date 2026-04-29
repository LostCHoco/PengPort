// PSP instance list 관리.
//
// 사용자는 여러 PengPort instance 에 동시 가입 가능 (Mastodon-like 분산 모델).
// localStorage 에 list + active id, keyring 에 instance 별 bearer token (lib/secrets.ts).
//
// schema:
// - localStorage 'pengport.instances'         : JSON [{ id, url, name?, addedAt }]
// - localStorage 'pengport.active_instance_id': string (현재 active 의 id)
// - keyring 'instance_token:<id>'             : 그 instance 의 bearer token

import { invoke } from "@tauri-apps/api/core";
import { instanceToken } from "./secrets";

const LS_INSTANCES = "pengport.instances";
const LS_ACTIVE_INSTANCE_ID = "pengport.active_instance_id";
const LS_LEGACY_INSTANCE_URL = "pengport.instance_url";

export interface InstanceEntry {
  id: string;
  url: string;
  /** 사용자가 표시용으로 지정한 이름. 없으면 instance metadata 의 name 사용. */
  name?: string;
  /** unix ms — list 정렬 / 표시용. */
  addedAt: number;
}

export function loadInstances(): InstanceEntry[] {
  const raw = localStorage.getItem(LS_INSTANCES);
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (x): x is InstanceEntry =>
        typeof x === "object" &&
        x !== null &&
        typeof (x as InstanceEntry).id === "string" &&
        typeof (x as InstanceEntry).url === "string",
    );
  } catch {
    return [];
  }
}

export function saveInstances(list: InstanceEntry[]): void {
  localStorage.setItem(LS_INSTANCES, JSON.stringify(list));
}

export function getActiveInstanceId(): string | null {
  return localStorage.getItem(LS_ACTIVE_INSTANCE_ID);
}

export function setActiveInstanceId(id: string | null): void {
  if (id === null) localStorage.removeItem(LS_ACTIVE_INSTANCE_ID);
  else localStorage.setItem(LS_ACTIVE_INSTANCE_ID, id);
}

export function getActiveInstance(): InstanceEntry | null {
  const id = getActiveInstanceId();
  if (!id) return null;
  return loadInstances().find((i) => i.id === id) ?? null;
}

/** 새 instance 추가. id 는 UUID 자동 발급. 동일 URL 이 이미 있으면 그 entry 재사용.
 * 기본적으로 active 로도 설정 (setActive: false 로 해제 가능). */
export function addInstance(input: {
  url: string;
  name?: string;
  setActive?: boolean;
}): InstanceEntry {
  const list = loadInstances();
  const existing = list.find((i) => i.url === input.url);
  if (existing) {
    if (input.setActive !== false) setActiveInstanceId(existing.id);
    return existing;
  }
  const entry: InstanceEntry = {
    id: crypto.randomUUID(),
    url: input.url,
    name: input.name,
    addedAt: Date.now(),
  };
  saveInstances([...list, entry]);
  if (input.setActive !== false) setActiveInstanceId(entry.id);
  return entry;
}

/** instance 삭제 + keyring token clear. active 였으면 첫 남은 instance 또는 null 로. */
export async function removeInstance(id: string): Promise<void> {
  const list = loadInstances();
  const next = list.filter((i) => i.id !== id);
  saveInstances(next);
  await instanceToken.clear(id);
  if (getActiveInstanceId() === id) {
    setActiveInstanceId(next[0]?.id ?? null);
  }
}

/** instance 의 표시 이름 갱신 (instance metadata fetch 후 자동 채움 또는 사용자 편집). */
export function updateInstanceName(id: string, name: string | undefined): void {
  const list = loadInstances();
  const next = list.map((i) => (i.id === id ? { ...i, name } : i));
  saveInstances(next);
}

/** 옛 단일 instance 모델 (pengport.instance_url + keyring 'instance_token') 데이터를
 * 새 list 의 첫 entry 로 변환. 한 번만 작동 (변환 후 옛 항목 정리).
 * 반환: 변환된 entry, 또는 옛 데이터 없으면 null. */
export async function migrateLegacy(): Promise<InstanceEntry | null> {
  const legacyUrl = localStorage.getItem(LS_LEGACY_INSTANCE_URL);
  if (!legacyUrl) return null;

  // idempotent — 이미 같은 URL 의 instance 가 list 에 있으면 옛 항목만 정리.
  const existing = loadInstances().find((i) => i.url === legacyUrl);
  if (existing) {
    localStorage.removeItem(LS_LEGACY_INSTANCE_URL);
    return existing;
  }

  const entry = addInstance({ url: legacyUrl });
  // keyring 의 옛 단일 'instance_token' entry → 새 'instance_token:<id>' 로 이전.
  await invoke<boolean>("instance_token_migrate_legacy", {
    newInstanceId: entry.id,
  });
  localStorage.removeItem(LS_LEGACY_INSTANCE_URL);
  return entry;
}

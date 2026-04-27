// Manifest / instance / catalog 인-메모리 TTL 캐시.
//
// 같은 세션 내 반복 fetch 방지. 앱 재시작 시 비워짐 (영구 캐시는 Phase 2).
//
// 사용 예시:
// ```ts
// const cached = manifestCache.get(serviceUrl);
// if (cached) return cached;
// const manifest = await pspLoadManifest(serviceUrl, token);
// manifestCache.set(serviceUrl, manifest);
// ```

import type { InstanceMetadata, ServiceManifest, ServicesCatalog } from "./types";

interface Entry<T> {
  data: T;
  fetchedAt: number;
}

export class TtlCache<K, V> {
  private map = new Map<K, Entry<V>>();
  constructor(private ttlMs: number) {}

  get(key: K): V | null {
    const e = this.map.get(key);
    if (!e) return null;
    if (Date.now() - e.fetchedAt > this.ttlMs) {
      this.map.delete(key);
      return null;
    }
    return e.data;
  }

  set(key: K, value: V) {
    this.map.set(key, { data: value, fetchedAt: Date.now() });
  }

  invalidate(key: K) {
    this.map.delete(key);
  }

  clear() {
    this.map.clear();
  }
}

/** Service manifest 캐시 — 5 분. 갱신은 invalidate 또는 TTL 만료 후. */
export const manifestCache = new TtlCache<string, ServiceManifest>(5 * 60_000);

/** Instance metadata 캐시 — 5 분. */
export const instanceCache = new TtlCache<string, InstanceMetadata>(5 * 60_000);

/** Services catalog 캐시 — 1 분 (자주 바뀜). */
export const catalogCache = new TtlCache<string, ServicesCatalog>(60_000);

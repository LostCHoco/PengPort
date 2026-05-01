// PengPort state 의 native + frontend 통합 정리.
//
// Settings 의 [데이터 초기화] / [PengPort 삭제] 와 ephemeral 모드 종료 자동 cleanup 둘 다 사용.

import { wipeAllData, type WipeReport } from "@/lib/api";
import { loadInstances } from "@/lib/instances";
import { catalogCache, instanceCache, manifestCache } from "@/lib/psp";

/**
 * native (keyring + 파일시스템) + frontend (localStorage + PSP 메모리 캐시) 를 한 번에 정리.
 *
 * native wipe 는 frontend 가 가진 instance id list / prism instance id list 를 넘긴다 — keyring
 * 은 enumerate API 가 없고, prism instance dir name 은 service id (운영자 catalog 가 source).
 *
 * 호출 후 React state 동기화 (active = null, refresh) 는 호출자가 책임 — context 의존이라
 * 이 헬퍼는 stateless 유지.
 */
export async function performWipe(): Promise<WipeReport> {
  const instanceIds = loadInstances().map((i) => i.id);
  const prismInstanceIds = collectPrismInstanceIdsFromCache();
  const report = await wipeAllData({ instanceIds, prismInstanceIds });

  // localStorage 의 PengPort 관련 항목 정리. 다른 namespace 항목은 안 건드림.
  localStorage.removeItem("pengport.instances");
  localStorage.removeItem("pengport.active_instance_id");
  localStorage.removeItem("pengport.instance_url");
  localStorage.removeItem("pengport.mode");
  localStorage.removeItem("pengport.sidebar.library_expanded");

  // PSP 메모리 캐시.
  catalogCache.clear();
  manifestCache.clear();
  instanceCache.clear();

  return report;
}

/**
 * PSP catalog cache 에 있는 service id 들 (= Prism instance dir name).
 * cache 가 비어있으면 (앱 시작 후 PspLibrary 미방문) 빈 배열 — Rust 측이 prism instance
 * 정리는 skip 하고 다른 state 만 wipe.
 */
function collectPrismInstanceIdsFromCache(): string[] {
  const seen = new Set<string>();
  for (const cat of catalogCache.values()) {
    for (const s of cat.services) {
      seen.add(s.id);
    }
  }
  return [...seen];
}

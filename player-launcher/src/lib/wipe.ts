// PengPort state 의 native + frontend 통합 정리.
//
// Settings 의 [데이터 초기화] 와 ephemeral 모드 종료 자동 cleanup 둘 다 사용.

import { wipeAllData, type ThirdPartyAppInstanceWipeTarget, type WipeReport } from "@/lib/api";
import { libraryList } from "@/lib/library";

/**
 * native (keyring + 파일시스템) + frontend (localStorage) 를 한 번에 정리.
 *
 * native wipe 는 라이브러리의 `third_party_app_launch` 레시피들을 app_id 별로 묶어
 * 인스턴스 dir name 후보로 넘긴다(레시피 id = 그 앱의 instance dir name). `spawn_process`
 * 레시피는 third-party app 인스턴스 폴더가 없어 대상에서 제외 — 그 데이터는 3단계
 * (APPDATA/LOCALAPPDATA 폴더 통째 삭제)가 어차피 다 지운다.
 *
 * 0.2.0: 인스턴스 개념이 없어져 keyring instance_token 정리 대상(`instanceIds`)은 항상
 * 빈 배열.
 *
 * 호출 후 React state 동기화는 호출자가 책임 — context 의존이라 이 헬퍼는 stateless 유지.
 */
export async function performWipe(): Promise<WipeReport> {
  const thirdPartyAppInstances = await libraryList()
    .then((recipes) => {
      const grouped = new Map<string, string[]>();
      for (const r of recipes) {
        if (r.launch.kind !== "third_party_app_launch") continue;
        const ids = grouped.get(r.launch.app_id) ?? [];
        ids.push(r.id);
        grouped.set(r.launch.app_id, ids);
      }
      return Array.from(grouped, ([appId, instanceIds]): ThirdPartyAppInstanceWipeTarget => ({
        appId,
        instanceIds,
      }));
    })
    .catch(() => [] as ThirdPartyAppInstanceWipeTarget[]);
  const report = await wipeAllData({ instanceIds: [], thirdPartyAppInstances });

  // localStorage 의 PengPort 관련 항목 정리. 다른 namespace 항목은 안 건드림.
  localStorage.removeItem("pengport.mode");

  return report;
}

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { checkForUpdate, type UpdateInfo } from "@/lib/updater";
import { catalogCache, instanceCache, manifestCache } from "@/lib/psp";
import { uninstallSelf, wipeAllData, type WipeReport } from "@/lib/api";
import { loadInstances } from "@/lib/instances";
import { useInstances } from "@/lib/instances-context";

type CheckState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "up_to_date"; currentVersion: string | null }
  | { kind: "available"; info: UpdateInfo }
  | { kind: "installing" }
  | { kind: "error"; message: string };

export default function Settings() {
  const [state, setState] = useState<CheckState>({ kind: "idle" });

  const onCheck = async () => {
    setState({ kind: "checking" });
    try {
      const info = await checkForUpdate();
      if (info.available) {
        setState({ kind: "available", info });
      } else {
        setState({ kind: "up_to_date", currentVersion: info.currentVersion });
      }
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  };

  const onInstall = async () => {
    if (state.kind !== "available" || !state.info.install) return;
    setState({ kind: "installing" });
    try {
      await state.info.install();
      // relaunch() 이후로 이 줄은 실행 안 됨
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  };

  return (
    <div className="space-y-6 p-8">
      <h2 className="text-2xl font-semibold">설정</h2>

      <section className="space-y-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5">
        <h3 className="text-sm font-medium text-neutral-200">업데이트</h3>

        <div className="flex items-center gap-3">
          <Button
            size="sm"
            onClick={onCheck}
            disabled={state.kind === "checking" || state.kind === "installing"}
          >
            {state.kind === "checking" ? "확인 중..." : "업데이트 확인"}
          </Button>

          {state.kind === "available" && (
            <Button size="sm" onClick={onInstall}>
              새 버전 설치 ({state.info.version})
            </Button>
          )}
        </div>

        {state.kind === "up_to_date" && (
          <p className="text-xs text-neutral-400">
            최신 버전입니다
            {state.currentVersion ? ` (v${state.currentVersion})` : ""}.
          </p>
        )}

        {state.kind === "available" && state.info.body && (
          <pre className="mt-2 whitespace-pre-wrap rounded bg-neutral-950 p-3 text-xs text-neutral-300">
            {state.info.body}
          </pre>
        )}

        {state.kind === "installing" && (
          <p className="text-xs text-neutral-400">
            다운로드 + 설치 중... 완료되면 자동으로 재시작됩니다.
          </p>
        )}

        {state.kind === "error" && (
          <p className="text-xs text-red-300" title={state.message}>
            실패:{" "}
            {state.message.length > 100
              ? state.message.slice(0, 100) + "..."
              : state.message}
          </p>
        )}
      </section>

      <InstancesSection />

      <DangerZone />
    </div>
  );
}

// ====== 인스턴스 관리 (등록한 PengPort 인스턴스 제거) ======

function InstancesSection() {
  const { instances, remove } = useInstances();

  const onRemove = async (id: string, label: string) => {
    const ok = confirm(
      `${label} 을(를) 제거할까요?\n\n` +
        `인스턴스 목록과 토큰이 사라집니다. 카탈로그/서비스 매니페스트는 다시 받게 됩니다.`,
    );
    if (!ok) return;
    await remove(id);
  };

  return (
    <section className="space-y-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5">
      <div className="flex items-baseline justify-between">
        <h3 className="text-sm font-medium text-neutral-200">인스턴스 관리</h3>
        <span className="text-[11px] text-neutral-500">등록된 PengPort 인스턴스</span>
      </div>

      <p className="text-xs text-neutral-400">
        제거하면 사이드바에서 사라지고, 다음에 같은 URL 로 다시 추가할 때 토큰을 새로
        입력해야 합니다.
      </p>

      {instances.length === 0 ? (
        <p className="text-xs text-neutral-500">등록된 인스턴스가 없습니다.</p>
      ) : (
        <ul className="space-y-2">
          {instances.map((inst) => {
            const label = inst.name ?? inst.url;
            return (
              <li
                key={inst.id}
                className="flex items-center justify-between gap-3 rounded border border-neutral-800 bg-neutral-950/40 px-3 py-2 text-xs"
              >
                <div className="min-w-0 flex-1">
                  <div className="truncate text-neutral-200">{label}</div>
                  <div className="mt-0.5 truncate font-mono text-[10.5px] text-neutral-500">
                    {inst.url}
                  </div>
                </div>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => void onRemove(inst.id, label)}
                >
                  제거
                </Button>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

// ====== 위험 영역 (데이터 초기화 / 프로그램 삭제) ======
//
// 두 작업 모두 비가역적. 단순 확인 다이얼로그로는 부족하다 — text 입력 (예: "초기화")
// 으로 자기 검증을 한 단계 더 강제. 친구 배포 시 실수로 누른 케이스 방지.

function DangerZone() {
  return (
    <section className="space-y-4 rounded-lg border border-red-900/40 bg-red-950/15 p-5">
      <div className="flex items-baseline justify-between">
        <h3 className="text-sm font-medium text-red-200">위험 영역</h3>
        <span className="text-[11px] text-red-300/70">비가역적 — 신중히 사용</span>
      </div>

      <ResetCard />
      <UninstallCard />
    </section>
  );
}

// --- 모든 데이터 초기화 ---

type ResetState =
  | { kind: "idle" }
  | { kind: "confirming" }
  | { kind: "running" }
  | { kind: "done"; report: WipeReport }
  | { kind: "error"; message: string };

function ResetCard() {
  const { instances, setActive, refresh } = useInstances();
  const [state, setState] = useState<ResetState>({ kind: "idle" });
  const [confirmText, setConfirmText] = useState("");

  const onStart = () => {
    setConfirmText("");
    setState({ kind: "confirming" });
  };

  const onCancel = () => {
    setState({ kind: "idle" });
    setConfirmText("");
  };

  const onConfirm = async () => {
    setState({ kind: "running" });
    try {
      // 1) Rust 측 native state wipe (keyring + files + prism instances).
      //    호출 직전에 instance ids / prism instance ids 를 모음.
      const liveInstances = loadInstances();
      const instanceIds = liveInstances.map((i) => i.id);
      // 현 PSP 카탈로그 service id 를 prism_instance_id 로 사용 — 이상적으로는 fetch 한
      // catalog 에서 가져와야 하지만, 대략 두 시나리오 대응:
      // (a) 자주 쓰는 service id 들을 후보로 시도 (없으면 무시).
      // (b) 직전 fetch 의 catalogCache 를 조회.
      // 단순화 — service id 를 프론트에서 알 수 없으니 instance id 를 그대로 시도.
      // PengPort 가 쓰는 prism instance dir name 은 service id (modded-mc, rlcraft-mc 등).
      // 미리 캐시된 카탈로그가 있으면 그 service ids 사용. 없으면 빈 배열 → Rust 가 skip.
      const prismInstanceIds = collectPrismInstanceIdsFromCache();
      const report = await wipeAllData({ instanceIds, prismInstanceIds });

      // 2) Frontend localStorage 정리.
      localStorage.removeItem("pengport.instances");
      localStorage.removeItem("pengport.active_instance_id");
      localStorage.removeItem("pengport.instance_url");

      // 3) PSP 메모리 캐시 정리 — fetch 한 catalog/manifest/instance 모두 비움.
      //    안 하면 사이드바/PspLibrary 가 stale 데이터 표시.
      catalogCache.clear();
      manifestCache.clear();
      instanceCache.clear();

      // 4) context state 동기화 — instances list + active 모두 빈 상태로.
      setActive(null);
      refresh();

      setState({ kind: "done", report });
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  };

  const allowConfirm = confirmText === "초기화";

  return (
    <div className="space-y-2 rounded border border-red-900/40 bg-neutral-900/40 p-4">
      <h4 className="text-sm text-neutral-100">모든 데이터 초기화</h4>
      <p className="text-xs text-neutral-400">
        등록한 인스턴스 ({instances.length}개), 토큰, 신뢰 목록, 자동 다운로드한
        PrismLauncher, PengPort 가 만든 Minecraft 인스턴스 폴더를 모두 삭제합니다.
        프로그램 자체는 그대로 유지되며, 다시 실행하면 처음 상태로 시작합니다.
      </p>

      {state.kind === "idle" && (
        <Button size="sm" variant="outline" onClick={onStart} className="cursor-pointer">
          초기화...
        </Button>
      )}

      {state.kind === "confirming" && (
        <div className="space-y-2 pt-1">
          <p className="text-xs text-red-300">
            계속하려면 아래에 <code className="font-mono">초기화</code> 라고
            입력하세요.
          </p>
          <input
            type="text"
            autoFocus
            value={confirmText}
            onChange={(e) => setConfirmText(e.target.value)}
            placeholder="초기화"
            className="w-40 rounded bg-neutral-950 px-2.5 py-1.5 text-xs text-neutral-100 outline-none ring-1 ring-neutral-800 focus:ring-red-700"
          />
          <div className="flex gap-2">
            <Button size="sm" variant="outline" onClick={onCancel} className="cursor-pointer">
              취소
            </Button>
            <Button
              size="sm"
              disabled={!allowConfirm}
              onClick={onConfirm}
              className="cursor-pointer bg-red-700 hover:bg-red-600 disabled:cursor-not-allowed"
            >
              실행
            </Button>
          </div>
        </div>
      )}

      {state.kind === "running" && (
        <p className="text-xs text-neutral-400">정리 중...</p>
      )}

      {state.kind === "done" && (
        <div className="space-y-1 text-xs">
          <p className="text-emerald-300">
            완료 — keyring {state.report.keyring_cleared}개,
            {state.report.paths_removed.length}개 폴더/파일 삭제.
          </p>
          {state.report.failures.length > 0 && (
            <p className="text-yellow-300/80">
              일부 실패 ({state.report.failures.length}개) — {state.report.failures[0]}
              {state.report.failures.length > 1 ? " 외" : ""}
            </p>
          )}
        </div>
      )}

      {state.kind === "error" && (
        <p className="text-xs text-red-300" title={state.message}>
          실패: {state.message}
        </p>
      )}
    </div>
  );
}

/**
 * PSP catalog cache 에 있는 service id 들 (= Prism instance dir name).
 * cache 가 비어있으면 (앱 시작 후 PspLibrary 미방문) 빈 배열 — Rust 측이 prism instance
 * 정리는 skip 하고 다른 state 만 wipe. 사용자가 wipe 직전 라이브러리를 한 번이라도
 * 봤으면 catalog 가 캐시되어 있어 prism 폴더도 같이 정리됨.
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

// --- PengPort 자체 삭제 ---

type UninstallState =
  | { kind: "idle" }
  | { kind: "confirming" }
  | { kind: "running" }
  | { kind: "error"; message: string };

function UninstallCard() {
  const [state, setState] = useState<UninstallState>({ kind: "idle" });
  const [confirmText, setConfirmText] = useState("");

  const onStart = () => {
    setConfirmText("");
    setState({ kind: "confirming" });
  };

  const onCancel = () => {
    setState({ kind: "idle" });
    setConfirmText("");
  };

  const onConfirm = async () => {
    setState({ kind: "running" });
    try {
      // uninstaller 가 PengPort 본체를 종료시키므로 이 호출 후 프로세스가 사라짐.
      await uninstallSelf();
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  };

  const allowConfirm = confirmText === "삭제";

  return (
    <div className="space-y-2 rounded border border-red-900/40 bg-neutral-900/40 p-4">
      <h4 className="text-sm text-neutral-100">PengPort 삭제</h4>
      <p className="text-xs text-neutral-400">
        OS 의 언인스톨러를 실행해 PengPort 자체를 제거합니다. 자동 다운로드된
        PrismLauncher 와 인스턴스 데이터 (Minecraft saves 등) 는{" "}
        <span className="font-medium text-neutral-300">남아 있을 수 있습니다</span>
        — 모두 지우려면 먼저 위의 "데이터 초기화" 를 실행한 뒤 삭제하세요.
      </p>

      {state.kind === "idle" && (
        <Button size="sm" variant="outline" onClick={onStart} className="cursor-pointer">
          PengPort 삭제...
        </Button>
      )}

      {state.kind === "confirming" && (
        <div className="space-y-2 pt-1">
          <p className="text-xs text-red-300">
            계속하려면 아래에 <code className="font-mono">삭제</code> 라고
            입력하세요. 언인스톨러가 실행되며 앱이 종료됩니다.
          </p>
          <input
            type="text"
            autoFocus
            value={confirmText}
            onChange={(e) => setConfirmText(e.target.value)}
            placeholder="삭제"
            className="w-40 rounded bg-neutral-950 px-2.5 py-1.5 text-xs text-neutral-100 outline-none ring-1 ring-neutral-800 focus:ring-red-700"
          />
          <div className="flex gap-2">
            <Button size="sm" variant="outline" onClick={onCancel} className="cursor-pointer">
              취소
            </Button>
            <Button
              size="sm"
              disabled={!allowConfirm}
              onClick={onConfirm}
              className="cursor-pointer bg-red-700 hover:bg-red-600 disabled:cursor-not-allowed"
            >
              실행
            </Button>
          </div>
        </div>
      )}

      {state.kind === "running" && (
        <p className="text-xs text-neutral-400">언인스톨러 실행 중...</p>
      )}

      {state.kind === "error" && (
        <p className="text-xs text-red-300" title={state.message}>
          실패: {state.message}
        </p>
      )}
    </div>
  );
}

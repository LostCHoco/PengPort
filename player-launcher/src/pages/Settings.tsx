import { useState } from "react";
import { Button } from "@/components/ui/button";
import { checkForUpdate, type UpdateInfo } from "@/lib/updater";
import type { WipeReport } from "@/lib/api";
import { performWipe } from "@/lib/wipe";
import { useMode } from "@/lib/mode-context";

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
      // 성공하면 프로세스가 새 exe 로 교체되며 종료되므로 이 줄은 실행 안 됨
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

      <ModeSection />

      <DangerZone />
    </div>
  );
}

// ====== 사용 모드 (1회용 vs 일반) ======

function ModeSection() {
  const { mode, setMode } = useMode();
  const [confirming, setConfirming] = useState<"to_ephemeral" | "to_normal" | null>(null);

  const onChangeRequest = (target: "to_ephemeral" | "to_normal") => {
    setConfirming(target);
  };

  const onConfirmChange = () => {
    if (confirming === "to_ephemeral") {
      // 일반 → 1회용. localStorage 저장 안 함 (mode-context 의 정책). 종료 시 wipe 활성.
      setMode("ephemeral");
    } else if (confirming === "to_normal") {
      // 1회용 → 일반. localStorage 에 저장. 종료 시 wipe 안 함.
      setMode("normal");
    }
    setConfirming(null);
  };

  const isEphemeral = mode === "ephemeral";

  return (
    <section
      className={`space-y-3 rounded-lg border ${
        isEphemeral
          ? "border-amber-900/60 bg-amber-950/15"
          : "border-neutral-800 bg-neutral-900/60"
      } p-5`}
    >
      <div className="flex items-baseline justify-between">
        <h3
          className={`text-sm font-medium ${
            isEphemeral ? "text-amber-200" : "text-neutral-200"
          }`}
        >
          사용 모드
        </h3>
        <span
          className={`text-[11px] ${
            isEphemeral ? "text-amber-300/70" : "text-neutral-500"
          }`}
        >
          현재: {isEphemeral ? "1회용 (공용 PC)" : "일반 (내 PC)"}
        </span>
      </div>

      {!isEphemeral ? (
        <>
          <p className="text-xs text-neutral-400">
            일반 모드 — 라이브러리 / third-party 앱 계정 / 실행 데이터가 영구 저장됩니다.
            공용 PC (PC방, 친구 PC) 에서 사용 중이면 1회용 모드로 변경 — 종료 시 모든 데이터와
            PengPort 자체가 자동 정리됩니다.
          </p>
          {confirming !== "to_ephemeral" && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => onChangeRequest("to_ephemeral")}
              className="cursor-pointer"
            >
              1회용 모드로 전환
            </Button>
          )}
          {confirming === "to_ephemeral" && (
            <div className="space-y-2 rounded border border-amber-900/40 bg-neutral-900/40 p-3">
              <p className="text-xs text-amber-200">
                1회용 모드로 전환하면 PengPort 종료 시 모든 데이터 + PengPort 자체가 자동 정리됩니다.
                자기 PC 라면 [취소] 하세요.
              </p>
              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setConfirming(null)}
                  className="cursor-pointer"
                >
                  취소
                </Button>
                <Button
                  size="sm"
                  onClick={onConfirmChange}
                  className="cursor-pointer bg-amber-700 hover:bg-amber-600"
                >
                  1회용으로 전환
                </Button>
              </div>
            </div>
          )}
        </>
      ) : (
        <>
          <p className="text-xs text-amber-200/90">
            1회용 모드 — 종료 시 모든 데이터 + PengPort 자체가 자동 정리됩니다. 자기 PC 라서
            데이터를 보존하고 싶다면 일반 모드로 변경하세요. 변경하면 종료 시 자동 정리 안 됩니다.
          </p>
          {confirming !== "to_normal" && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => onChangeRequest("to_normal")}
              className="cursor-pointer"
            >
              일반 모드로 전환
            </Button>
          )}
          {confirming === "to_normal" && (
            <div className="space-y-2 rounded border border-neutral-700 bg-neutral-900/40 p-3">
              <p className="text-xs text-neutral-300">
                일반 모드로 전환 — 현재 데이터 (라이브러리 / third-party 앱 계정 등) 가 영구
                저장됩니다. 공용 PC 라면 [취소] 하세요.
              </p>
              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setConfirming(null)}
                  className="cursor-pointer"
                >
                  취소
                </Button>
                <Button size="sm" onClick={onConfirmChange} className="cursor-pointer">
                  일반 모드로 전환
                </Button>
              </div>
            </div>
          )}
        </>
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
      // native + frontend 통합 wipe — keyring, 파일시스템, localStorage.
      const report = await performWipe();
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
        라이브러리, 자동 다운로드한 third-party 앱, PengPort 가 만든 실행 데이터 폴더를
        모두 삭제합니다. 프로그램 자체는 그대로 유지되며, 다시 실행하면 처음
        상태로 시작합니다.
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
              className="cursor-pointer bg-red-700 hover:bg-red-600"
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

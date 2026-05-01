import { useState } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { Button } from "@/components/ui/button";
import { checkForUpdate, type UpdateInfo } from "@/lib/updater";
import { uninstallSelf, type WipeReport } from "@/lib/api";
import { useInstances } from "@/lib/instances-context";
import { instanceToken } from "@/lib/secrets";
import { buildInviteLandingUrl } from "@/lib/invite";
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

      <ModeSection />

      <DangerZone />
    </div>
  );
}

// ====== 사용 모드 (1회용 vs 평소) ======

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
          현재: {isEphemeral ? "1회용 (공용 PC)" : "평소 (내 PC)"}
        </span>
      </div>

      {!isEphemeral ? (
        <>
          <p className="text-xs text-neutral-400">
            평소 모드 — 인스턴스 / 토큰 / Prism 계정 / 게임 세이브가 영구 저장됩니다.
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
            데이터를 보존하고 싶다면 평소 모드로 변경하세요. 변경하면 종료 시 자동 정리 안 됩니다.
          </p>
          {confirming !== "to_normal" && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => onChangeRequest("to_normal")}
              className="cursor-pointer"
            >
              평소 모드로 전환
            </Button>
          )}
          {confirming === "to_normal" && (
            <div className="space-y-2 rounded border border-neutral-700 bg-neutral-900/40 p-3">
              <p className="text-xs text-neutral-300">
                평소 모드로 전환 — 현재 데이터 (인스턴스 / 토큰 등) 가 영구 저장됩니다. 공용 PC
                라면 [취소] 하세요.
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
                  평소 모드로 전환
                </Button>
              </div>
            </div>
          )}
        </>
      )}
    </section>
  );
}

// ====== 인스턴스 관리 (등록한 PengPort 인스턴스 제거 + 초대 링크 복사) ======

type CopyState = { id: string; kind: "ok" | "error"; message: string } | null;

function InstancesSection() {
  const { instances, remove } = useInstances();
  const [copyState, setCopyState] = useState<CopyState>(null);

  const onRemove = async (id: string, label: string) => {
    const ok = confirm(
      `${label} 을(를) 제거할까요?\n\n` +
        `인스턴스 목록과 토큰이 사라집니다. 카탈로그/서비스 매니페스트는 다시 받게 됩니다.`,
    );
    if (!ok) return;
    await remove(id);
  };

  // 초대 링크 복사: 그 인스턴스의 keyring token 으로 `pengport://join?...` 생성.
  // 토큰이 없으면 (auth.type=none 이거나 미저장) token=빈 값으로 생성.
  // 사용자에게 "토큰이 평문 포함" 경고를 한 번 보여주고 진행 — 친구 그룹 / 반신뢰 모델 가정.
  const onCopyInvite = async (id: string, url: string) => {
    setCopyState(null);
    try {
      const token = (await instanceToken.load(id)) ?? "";
      if (token.length > 0) {
        const ok = confirm(
          "이 초대 링크에는 인스턴스 토큰이 평문으로 포함됩니다.\n\n" +
            "신뢰하는 친구에게만 1:1 로 전달하세요. 공개 채널 (오픈 채팅, 공개 게시판 등)\n" +
            "에 올리지 마세요.\n\n계속할까요?",
        );
        if (!ok) return;
      }
      // HTTPS landing 형식 — 디스코드/카톡 등 메시지 앱에서 자동 hyperlink.
      // gateway 의 `/invite` 가 meta refresh 로 `pengport://join?...` 로 redirect.
      const inviteUrl = buildInviteLandingUrl({ url, token });
      await writeText(inviteUrl);
      setCopyState({ id, kind: "ok", message: "초대 링크 복사됨 (디스코드/카톡 가능)" });
    } catch (e) {
      setCopyState({ id, kind: "error", message: String(e) });
    }
  };

  return (
    <section className="space-y-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5">
      <div className="flex items-baseline justify-between">
        <h3 className="text-sm font-medium text-neutral-200">인스턴스 관리</h3>
        <span className="text-[11px] text-neutral-500">등록된 PengPort 인스턴스</span>
      </div>

      <p className="text-xs text-neutral-400">
        제거하면 사이드바에서 사라지고, 다음에 같은 URL 로 다시 추가할 때 토큰을 새로
        입력해야 합니다. <span className="text-neutral-300">초대 링크</span> 는 친구가
        클릭하면 PengPort 가 자동으로 가입 dialog 를 띄웁니다.
      </p>

      {instances.length === 0 ? (
        <p className="text-xs text-neutral-500">등록된 인스턴스가 없습니다.</p>
      ) : (
        <ul className="space-y-2">
          {instances.map((inst) => {
            const label = inst.name ?? inst.url;
            const note = copyState?.id === inst.id ? copyState : null;
            return (
              <li
                key={inst.id}
                className="space-y-1 rounded border border-neutral-800 bg-neutral-950/40 px-3 py-2 text-xs"
              >
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-neutral-200">{label}</div>
                    <div className="mt-0.5 truncate font-mono text-[10.5px] text-neutral-500">
                      {inst.url}
                    </div>
                  </div>
                  <div className="flex gap-2">
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => void onCopyInvite(inst.id, inst.url)}
                    >
                      초대 링크 복사
                    </Button>
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() => void onRemove(inst.id, label)}
                    >
                      제거
                    </Button>
                  </div>
                </div>
                {note && (
                  <p
                    className={
                      note.kind === "ok"
                        ? "text-[11px] text-emerald-300"
                        : "text-[11px] text-red-300"
                    }
                  >
                    {note.kind === "ok" ? "✓ " : "✗ "}
                    {note.message}
                  </p>
                )}
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
      // native + frontend 통합 wipe — keyring, 파일시스템, localStorage, PSP 캐시.
      const report = await performWipe();
      // context state 동기화 — instances list + active 모두 빈 상태로.
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

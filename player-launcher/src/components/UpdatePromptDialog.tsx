// 자동 업데이트 동의 dialog.
//
// 앱 시작 시 새 버전이 발견되면 App.tsx 가 띄움. 사용자가 "지금 업데이트" 누르면 install +
// 자동 재시작, "다음에" 누르면 그 세션 동안 표시 안 함 (다음 launch 시 또 묻음).
//
// 0.1.3 이전엔 silent auto-install 이었지만 사용자가 모르는 사이 재시작되는 게 어색해서
// 0.1.3 부터 명시 동의 받음.

import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";
import { useDraggablePosition } from "@/lib/use-draggable-position";

export interface UpdatePromptInfo {
  version: string;
  currentVersion: string | null;
  body: string | null;
  install: () => Promise<void>;
}

interface Props {
  /** null 이면 dialog 닫힘. */
  info: UpdatePromptInfo | null;
  /** "다음에" 클릭 — 그 세션 동안만 닫음 (다음 launch 에 또 표시). */
  onDismiss: () => void;
}

type Phase =
  | { kind: "idle" }
  | { kind: "installing" }
  | { kind: "error"; message: string };

export function UpdatePromptDialog({ info, onDismiss }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const { style: dragStyle, onHeaderMouseDown } = useDraggablePosition(info !== null);

  // info 가 새로 들어올 때마다 phase reset.
  useEffect(() => {
    if (info) setPhase({ kind: "idle" });
  }, [info]);

  // ESC 닫기 (설치 중에는 무시).
  useEffect(() => {
    if (!info) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && phase.kind !== "installing") onDismiss();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [info, phase.kind, onDismiss]);

  // 첫 포커스 — "다음에" (안전한 기본값).
  useEffect(() => {
    if (!info || phase.kind !== "idle") return;
    cardRef.current
      ?.querySelector<HTMLButtonElement>("[data-update-dismiss]")
      ?.focus();
  }, [info, phase.kind]);

  if (!info) return null;

  const handleInstall = async () => {
    setPhase({ kind: "installing" });
    try {
      await info.install();
      // 성공하면 프로세스가 새 exe 로 교체되며 종료되므로 이 줄은 도달 안 함.
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  };

  return (
    <Portal>
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="update-prompt-title"
    >
      <div
        ref={cardRef}
        className="w-full max-w-md rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
        style={dragStyle}
        onClick={(e) => e.stopPropagation()}
      >
        <h3
          id="update-prompt-title"
          className="text-lg font-semibold text-neutral-50"
          onMouseDown={onHeaderMouseDown}
        >
          새 버전이 있습니다
        </h3>

        <div className="mt-3 space-y-3 text-sm text-neutral-300">
          <p>
            현재{" "}
            <span className="font-mono text-neutral-400">
              v{info.currentVersion ?? "?"}
            </span>{" "}
            →{" "}
            <span className="font-mono font-medium text-emerald-300">
              v{info.version}
            </span>
          </p>

          {info.body && info.body.trim() && (
            <pre className="max-h-40 overflow-y-auto whitespace-pre-wrap rounded bg-neutral-950 p-3 text-xs text-neutral-300">
              {info.body}
            </pre>
          )}

          <p className="text-xs text-neutral-500">
            업데이트 시 자동 다운로드 + 서명 검증 + 재시작. 진행 중인 게임은 영향 없습니다.
          </p>

          {phase.kind === "installing" && (
            <p className="text-xs text-neutral-400">
              다운로드 + 설치 중... 완료되면 PengPort 가 자동으로 다시 시작됩니다.
            </p>
          )}

          {phase.kind === "error" && (
            <p className="text-xs text-red-300" title={phase.message}>
              실패:{" "}
              {phase.message.length > 200
                ? phase.message.slice(0, 200) + "..."
                : phase.message}
            </p>
          )}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <Button
            data-update-dismiss
            variant="outline"
            size="sm"
            disabled={phase.kind === "installing"}
            onClick={onDismiss}
            className="cursor-pointer"
          >
            다음에
          </Button>
          <Button
            size="sm"
            disabled={phase.kind === "installing"}
            onClick={handleInstall}
            className="min-w-[110px] cursor-pointer"
          >
            {phase.kind === "installing" ? (
              <span className="inline-flex items-center gap-1.5">
                <Spinner />
                설치 중
              </span>
            ) : phase.kind === "error" ? (
              "다시 시도"
            ) : (
              "지금 업데이트"
            )}
          </Button>
        </div>
      </div>
    </div>
    </Portal>
  );
}

function Spinner() {
  return (
    <span
      className="h-3 w-3 animate-spin rounded-full border-2 border-current border-t-transparent"
      aria-label="설치 중"
    />
  );
}

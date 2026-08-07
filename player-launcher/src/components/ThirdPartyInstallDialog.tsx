// 레시피 실행 직전, 대상 third-party app 이 시스템에 없을 때 띄우는 inline 설치 dialog.
// 사용자가 "설치하기" 누르면 자동 다운로드 후 원래 레시피 자동 재실행 — 페이지 이동 없이 처리.
//
// 백엔드의 `LaunchOutcome.kind === "third_party_app_missing"` 을 받았을 때 부모
// (Library.tsx) 가 그 outcome 의 `app_id` 를 넘겨 띄움 — app_id 하나로 등록된 모든
// third-party app 을 다루는 범용 다이얼로그(옛 "항상 PrismLauncher" 하드코딩 폐지).

import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";
import { downloadThirdPartyApp, listThirdPartyApps } from "@/lib/api";
import { useDraggablePosition } from "@/lib/use-draggable-position";

interface Props {
  /** 설치 대상 third-party app id — null 이면 다이얼로그 닫힘. */
  appId: string | null;
  /** 설치 완료 후 호출 — 부모가 원래 실행을 재시도. */
  onInstalled: () => void;
  /** 사용자가 취소 — dialog 닫음. */
  onCancel: () => void;
}

type Phase = { kind: "ask" } | { kind: "downloading" } | { kind: "error"; message: string };

export function ThirdPartyInstallDialog({ appId, onInstalled, onCancel }: Props) {
  const open = appId !== null;
  const cardRef = useRef<HTMLDivElement>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "ask" });
  const [label, setLabel] = useState<string | null>(null);
  const { style: dragStyle, onHeaderMouseDown } = useDraggablePosition(open);

  useEffect(() => {
    if (open) setPhase({ kind: "ask" });
  }, [open]);

  useEffect(() => {
    if (!appId) return;
    let cancelled = false;
    void listThirdPartyApps().then((apps) => {
      if (cancelled) return;
      setLabel(apps.find((a) => a.id === appId)?.label ?? appId);
    });
    return () => {
      cancelled = true;
    };
  }, [appId]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && phase.kind !== "downloading") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, phase.kind, onCancel]);

  useEffect(() => {
    if (!open || phase.kind !== "ask") return;
    cardRef.current?.querySelector<HTMLButtonElement>("[data-tp-cancel]")?.focus();
  }, [open, phase.kind]);

  if (!appId) return null;

  const handleInstall = async () => {
    setPhase({ kind: "downloading" });
    try {
      await downloadThirdPartyApp(appId);
      onInstalled();
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
      aria-labelledby="tp-install-title"
    >
      <div
        ref={cardRef}
        className="w-full max-w-md rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
        style={dragStyle}
        onClick={(e) => e.stopPropagation()}
      >
        <h3
          id="tp-install-title"
          className="text-lg font-semibold text-neutral-50"
          onMouseDown={onHeaderMouseDown}
        >
          {label ?? appId} 가 필요합니다
        </h3>

        <div className="mt-3 space-y-3 text-sm text-neutral-300">
          <p>
            이 앱은 <span className="font-medium text-neutral-100">{label ?? appId}</span> 로
            실행됩니다. 시스템에서 찾을 수 없어 자동 다운로드가 필요합니다.
          </p>

          <div className="rounded border border-neutral-800 bg-neutral-950/40 px-3 py-2.5 text-xs text-neutral-400">
            <p>
              PengPort 가 전용 폴더 (
              <code className="text-neutral-300">%LOCALAPPDATA%\PengPort\{appId}\</code>) 에
              받아 격리합니다. 시스템에 이미 설치된 사본과 분리되며 언제든 [서드파티 앱]
              페이지에서 삭제할 수 있습니다.
            </p>
          </div>

          {phase.kind === "downloading" && (
            <p className="text-xs text-neutral-400">
              다운로드 + 설치 중... (수십 MB, 30초 ~ 2분)
            </p>
          )}

          {phase.kind === "error" && (
            <p className="text-xs text-red-300" title={phase.message}>
              실패:{" "}
              {phase.message.length > 200 ? phase.message.slice(0, 200) + "..." : phase.message}
            </p>
          )}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <Button
            data-tp-cancel
            variant="outline"
            size="sm"
            disabled={phase.kind === "downloading"}
            onClick={onCancel}
            className="cursor-pointer"
          >
            취소
          </Button>
          <Button
            size="sm"
            disabled={phase.kind === "downloading"}
            onClick={() => void handleInstall()}
            className="min-w-[100px] cursor-pointer"
          >
            {phase.kind === "downloading" ? (
              <span className="inline-flex items-center gap-1.5">
                <Spinner />
                설치 중
              </span>
            ) : phase.kind === "error" ? (
              "다시 시도"
            ) : (
              "설치하기"
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

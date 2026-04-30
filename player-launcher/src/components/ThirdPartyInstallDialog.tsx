// Service action 실행 직전, 필요한 third-party app 이 시스템에 없을 때 띄우는
// inline 동의 dialog. 사용자가 "설치하기" 누르면 자동 다운로드 후 원래 action
// 자동 retry — 페이지 이동 없이 처리.
//
// 백엔드의 ActionOutcome::ThirdPartyMissing 을 받았을 때 부모(PspLibrary) 가 띄움.
// 현 Phase 1 카탈로그 = prism-launcher 만. 다른 app 추가되면 분기 추가.

import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { downloadPrism } from "@/lib/api";
import type { InstallHintDto } from "@/lib/psp";

export interface InstallRequest {
  app_id: string;
  install_hint: InstallHintDto | null;
}

interface Props {
  /** null 이면 dialog 닫힘. */
  request: InstallRequest | null;
  /** 설치 완료 후 호출 — 부모가 원래 action 재시도. */
  onInstalled: (req: InstallRequest) => void;
  /** 사용자가 취소 — dialog 닫음. */
  onCancel: () => void;
}

type Phase =
  | { kind: "ask" }
  | { kind: "downloading" }
  | { kind: "error"; message: string };

export function ThirdPartyInstallDialog({
  request,
  onInstalled,
  onCancel,
}: Props) {
  const cardRef = useRef<HTMLDivElement>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "ask" });

  // request 가 새로 들어올 때마다 phase reset.
  useEffect(() => {
    if (request) setPhase({ kind: "ask" });
  }, [request]);

  // ESC 닫기 (다운로드 중에는 무시 — 중간에 끊으면 부분 설치 발생).
  useEffect(() => {
    if (!request) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && phase.kind !== "downloading") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [request, phase.kind, onCancel]);

  // 첫 포커스 — "취소" 가 안전한 기본값.
  useEffect(() => {
    if (!request || phase.kind !== "ask") return;
    cardRef.current
      ?.querySelector<HTMLButtonElement>("[data-tp-cancel]")
      ?.focus();
  }, [request, phase.kind]);

  if (!request) return null;

  const handleInstall = async () => {
    setPhase({ kind: "downloading" });
    try {
      if (request.app_id === "prism-launcher") {
        await downloadPrism();
      } else {
        throw new Error(`미지원 third-party app: ${request.app_id}`);
      }
      onInstalled(request);
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  };

  const handleBackdrop = () => {
    if (phase.kind !== "downloading") onCancel();
  };

  const appName = appLabel(request.app_id, request.install_hint?.name);
  const homepage = request.install_hint?.homepage;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onClick={handleBackdrop}
      role="dialog"
      aria-modal="true"
      aria-labelledby="tp-install-title"
    >
      <div
        ref={cardRef}
        className="w-full max-w-md rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3
          id="tp-install-title"
          className="text-lg font-semibold text-neutral-50"
        >
          {appName} 가 필요합니다
        </h3>

        <div className="mt-3 space-y-3 text-sm text-neutral-300">
          <p>
            이 서비스는{" "}
            <span className="font-medium text-neutral-100">{appName}</span> 로
            실행됩니다. 시스템에서 찾을 수 없어 자동 다운로드가 필요합니다.
          </p>

          <div className="rounded border border-neutral-800 bg-neutral-950/40 px-3 py-2.5 text-xs text-neutral-400">
            <p>
              PengPort 가 전용 폴더 (
              <code className="text-neutral-300">%LOCALAPPDATA%\app.pengport\prism\</code>
              ) 에 받아 격리합니다. 시스템 Prism 데이터와 분리되며 언제든 [서드파티
              앱] 페이지에서 삭제할 수 있습니다.
            </p>
            {homepage && (
              <p className="mt-1.5">
                공식 홈페이지:{" "}
                <span className="font-mono text-neutral-300">{homepage}</span>
              </p>
            )}
          </div>

          {phase.kind === "downloading" && (
            <p className="text-xs text-neutral-400">
              다운로드 + 설치 중... (수십 MB, 30초 ~ 2분)
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
            onClick={handleInstall}
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
  );
}

function appLabel(appId: string, hintName: string | undefined): string {
  if (hintName) return hintName;
  if (appId === "prism-launcher") return "PrismLauncher";
  return appId;
}

function Spinner() {
  return (
    <span
      className="h-3 w-3 animate-spin rounded-full border-2 border-current border-t-transparent"
      aria-label="설치 중"
    />
  );
}

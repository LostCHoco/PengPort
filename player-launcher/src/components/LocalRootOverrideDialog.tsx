// 로컬 경로 오버라이드 다이얼로그 — 이미 다른 위치에 설치돼 있는 앱을 PengPort가 직접
// 설치하는 대신 그 폴더를 그대로 쓰도록 "연결"만 한다. 라이브러리에서 제거해도 이
// 폴더 자체는 지우지 않는다(`Library.tsx`의 `deleteInstalledDataTolerant` 참고 — override
// 가 설정된 항목은 파일 삭제를 건너뛰고 라이브러리에서만 뺀다).
//
// `OptionalGroupsDialog.tsx`와 같은 손수-만든 modal 패턴.

import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";
import { libraryGetLocalRootOverride, librarySetLocalRootOverride } from "@/lib/library";
import type { Recipe } from "@/lib/library";

interface Props {
  /** null 이면 닫힘. */
  recipe: Recipe | null;
  onClose: () => void;
}

type Phase =
  | { kind: "loading" }
  | { kind: "ready"; path: string | null }
  | { kind: "saving" }
  | { kind: "error"; message: string };

export function LocalRootOverrideDialog({ recipe, onClose }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });

  useEffect(() => {
    if (!recipe) return;
    let cancelled = false;
    setPhase({ kind: "loading" });
    (async () => {
      try {
        const path = await libraryGetLocalRootOverride(recipe.id);
        if (!cancelled) setPhase({ kind: "ready", path });
      } catch (e) {
        if (!cancelled) setPhase({ kind: "error", message: String(e) });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [recipe]);

  useEffect(() => {
    if (!recipe) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [recipe, onClose]);

  if (!recipe) return null;

  const onPickFolder = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        directory: true,
        multiple: false,
        title: `${recipe.name} 이 설치돼 있는 폴더 선택`,
      });
      if (!picked || typeof picked !== "string") return;
      setPhase({ kind: "saving" });
      await librarySetLocalRootOverride(recipe.id, picked);
      setPhase({ kind: "ready", path: picked });
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  };

  const onClear = async () => {
    setPhase({ kind: "saving" });
    try {
      await librarySetLocalRootOverride(recipe.id, null);
      setPhase({ kind: "ready", path: null });
    } catch (e) {
      setPhase({ kind: "error", message: String(e) });
    }
  };

  const currentPath = phase.kind === "ready" ? phase.path : null;

  return (
    <Portal>
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
      aria-labelledby="local-root-override-title"
    >
      <div
        ref={cardRef}
        className="w-full max-w-md rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 id="local-root-override-title" className="text-lg font-semibold text-neutral-50">
          {recipe.name} — 로컬 폴더 연결
        </h3>
        <p className="mt-1 text-xs text-neutral-500">
          이미 다른 위치에 설치돼 있으면 그 폴더를 연결해 PengPort 가 새로 설치하지 않고
          그대로 쓰게 할 수 있습니다. 라이브러리에서 제거해도 이 폴더는 지워지지 않습니다.
        </p>

        <div className="mt-4 rounded border border-neutral-800 bg-neutral-950/60 p-3 text-xs">
          {phase.kind === "loading" && <p className="text-neutral-400">불러오는 중...</p>}
          {phase.kind === "error" && (
            <p className="text-red-300" title={phase.message}>
              실패: {phase.message}
            </p>
          )}
          {(phase.kind === "ready" || phase.kind === "saving") && (
            <p className={currentPath ? "break-all text-neutral-200" : "text-neutral-500"}>
              {currentPath ?? "연결 안 됨 — PengPort 가 자동 관리하는 위치에 설치됩니다."}
            </p>
          )}
        </div>

        <div className="mt-6 flex justify-between gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={phase.kind === "saving" || !currentPath}
            onClick={onClear}
            className="cursor-pointer"
          >
            연결 해제
          </Button>
          <div className="flex gap-2">
            <Button variant="outline" size="sm" onClick={onClose} className="cursor-pointer">
              닫기
            </Button>
            <Button
              size="sm"
              disabled={phase.kind === "saving"}
              onClick={onPickFolder}
              className="cursor-pointer"
            >
              폴더 선택
            </Button>
          </div>
        </div>
      </div>
    </div>
    </Portal>
  );
}

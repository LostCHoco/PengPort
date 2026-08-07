// "업데이트 필요" 뱃지를 눌렀을 때 — 어느 항목(압축/파일)이 아직 반영된 적 없는지
// 보여주는 dialog. `library_install_status`(뱃지 자체, 개수만)와 달리 사용자가 뱃지를
// 눌렀을 때만 온디맨드로 조회한다.
//
// 항목 내용의 "어느 부분이 다른지"까지는 안 보여준다 — 설치 이후 앱 사용으로 생기는
// 정상적인 변화(런타임 캐시/설정)와 진짜 레시피 변경을 실제 파일 비교로는 구분할 수
// 없다는 게 이 세션에서 확인된 근본 이유. "아직 반영된 적 없다"는 원장 사실만
// 정직하게 보여준다.
//
// `ThirdPartyInstallDialog.tsx`와 같은 손수-만든 modal 패턴(이 프로젝트엔 shadcn Dialog
// 프리미티브가 없음).

import { useEffect, useRef, useState } from "react";
import { Portal } from "@/components/ui/portal";
import { libraryInstallDiagnostics } from "@/lib/library";
import type { InstallDiagnostic, RecipeSummary } from "@/lib/library";
import { useDraggablePosition } from "@/lib/use-draggable-position";

interface Props {
  /** null 이면 닫힘. 값이 있으면 그 레시피의 진단을 조회해서 연다. */
  recipe: RecipeSummary | null;
  onClose: () => void;
}

type Phase =
  | { kind: "loading" }
  | { kind: "loaded"; diagnostics: InstallDiagnostic[] }
  | { kind: "error"; message: string };

export function InstallDiffDialog({ recipe, onClose }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });
  const { style: dragStyle, onHeaderMouseDown } = useDraggablePosition(recipe !== null);

  useEffect(() => {
    if (!recipe) return;
    let cancelled = false;
    setPhase({ kind: "loading" });
    (async () => {
      try {
        const diagnostics = await libraryInstallDiagnostics(recipe.id);
        if (!cancelled) setPhase({ kind: "loaded", diagnostics });
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

  useEffect(() => {
    if (recipe) cardRef.current?.querySelector<HTMLButtonElement>("[data-diff-close]")?.focus();
  }, [recipe]);

  if (!recipe) return null;

  return (
    <Portal>
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="install-diff-title"
    >
      <div
        ref={cardRef}
        className="flex max-h-[80vh] w-full max-w-lg flex-col rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
        style={dragStyle}
        onClick={(e) => e.stopPropagation()}
      >
        <h3
          id="install-diff-title"
          className="text-lg font-semibold text-neutral-50"
          onMouseDown={onHeaderMouseDown}
        >
          {recipe.name} — 왜 업데이트가 필요한가요?
        </h3>
        <p className="mt-1 text-xs text-neutral-500">
          아직 지금 레시피 내용대로 반영된 적 없는 항목입니다. 설치 이후 앱을 쓰면서 생긴
          변화(캐시·앱 안에서 바꾼 설정 등)는 여기 안 뜹니다 — 그건 레시피 책임 밖입니다.
        </p>

        <div className="mt-4 min-h-0 flex-1 overflow-y-auto">
          {phase.kind === "loading" && (
            <p className="text-sm text-neutral-400">확인하는 중...</p>
          )}
          {phase.kind === "error" && (
            <p className="text-sm text-red-300">확인 실패: {phase.message}</p>
          )}
          {phase.kind === "loaded" && phase.diagnostics.length === 0 && (
            <p className="text-sm text-neutral-400">
              반영 안 된 항목을 못 찾았습니다 — 방금 반영됐을 수 있습니다.
            </p>
          )}
          {phase.kind === "loaded" && phase.diagnostics.length > 0 && (
            <ul className="space-y-2">
              {phase.diagnostics.map((d, i) => (
                <li
                  key={i}
                  className="rounded border border-neutral-800 bg-neutral-950/40 p-3"
                >
                  <DiagnosticItem diagnostic={d} />
                </li>
              ))}
            </ul>
          )}
        </div>

        <div className="mt-6 flex justify-end">
          <button
            data-diff-close
            type="button"
            onClick={onClose}
            className="cursor-pointer rounded-md border border-neutral-700 px-3 py-1.5 text-sm text-neutral-200 transition-colors hover:bg-neutral-800/60"
          >
            닫기
          </button>
        </div>
      </div>
    </div>
    </Portal>
  );
}

function DiagnosticItem({ diagnostic }: { diagnostic: InstallDiagnostic }) {
  if (diagnostic.kind === "needs_optional_group_selection") {
    return (
      <p className="text-xs text-neutral-400">
        새로 추가되거나 바뀐 설치 옵션이 있어 아직 확인이 필요합니다 — "업데이트"를 누르면
        무엇을 받을지 고르는 선택 창이 먼저 뜹니다.
      </p>
    );
  }
  if (diagnostic.kind === "archive_pending") {
    return (
      <>
        <p className="truncate font-mono text-xs text-neutral-300" title={diagnostic.url}>
          {diagnostic.url}
        </p>
        <p className="mt-1 text-xs text-neutral-400">압축 다운로드 — 아직 반영된 적 없음.</p>
        {diagnostic.missing_paths.length === 0 ? (
          <p className="mt-1 text-xs text-amber-300">
            단, 선언된 파일은 지금 디스크에 전부 있습니다 — 기록(마커)만 없는 상태로
            보입니다. "업데이트"를 눌러도 실제로는 다시 받기만 할 뿐 내용은 그대로일
            가능성이 높습니다.
          </p>
        ) : (
          <div className="mt-1 space-y-0.5">
            <p className="text-xs text-neutral-500">실제로 없는 파일 {diagnostic.missing_paths.length}개:</p>
            <ul className="max-h-32 space-y-0.5 overflow-y-auto">
              {diagnostic.missing_paths.map((path) => (
                <li key={path} className="truncate font-mono text-[11px] text-neutral-500" title={path}>
                  {path}
                </li>
              ))}
            </ul>
          </div>
        )}
      </>
    );
  }
  return (
    <>
      <p className="truncate font-mono text-xs text-neutral-300" title={diagnostic.path}>
        {diagnostic.path}
      </p>
      <p className="mt-1 text-xs text-neutral-400">파일 오버라이드 — 아직 반영된 적 없음.</p>
    </>
  );
}

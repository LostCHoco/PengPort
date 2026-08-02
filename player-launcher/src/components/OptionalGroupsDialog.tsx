// 선택적 그룹(부분 설치) 확인 다이얼로그 — `Recipe.optional_groups`가 있는 레시피는
// "설치" 버튼을 누를 때마다 이 다이얼로그가 먼저 뜬다(그룹이 아예 없는 레시피만
// 건너뜀).
//
// 레시피가 `default_selected: true`로 선언해도 자동으로 설치되지 않는다 — 여기서
// 사용자가 확인 버튼을 눌러야만 실제로 반영된다(레시피 기본값은 체크박스를 미리
// 채우는 용도일 뿐). 확인하면 그 즉시 `libraryInstall`을 호출해 재조정 — 켠 그룹은
// 압축에서 복원되고, 끈 그룹은 기존 화이트리스트 정리 로직이 지운다.
//
// `ThirdPartyInstallDialog.tsx`와 같은 손수-만든 modal 패턴.

import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";
import { libraryGetSelectedOptionalGroups } from "@/lib/library";
import type { Recipe } from "@/lib/library";

interface Props {
  /** null 이면 닫힘. */
  recipe: Recipe | null;
  onConfirm: (groups: string[]) => void;
  onCancel: () => void;
}

type Phase = { kind: "loading" } | { kind: "ready" } | { kind: "error"; message: string };

export function OptionalGroupsDialog({ recipe, onConfirm, onCancel }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "loading" });
  const [checked, setChecked] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (!recipe) return;
    let cancelled = false;
    setPhase({ kind: "loading" });
    (async () => {
      try {
        const stored = await libraryGetSelectedOptionalGroups(recipe.id);
        if (cancelled) return;
        const initial = new Set(
          stored ?? recipe.optional_groups.filter((g) => g.default_selected).map((g) => g.id),
        );
        setChecked(initial);
        setPhase({ kind: "ready" });
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
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [recipe, onCancel]);

  useEffect(() => {
    if (recipe) cardRef.current?.querySelector<HTMLButtonElement>("[data-groups-confirm]")?.focus();
  }, [recipe, phase.kind]);

  if (!recipe) return null;

  const toggle = (id: string) => {
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <Portal>
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onClick={onCancel}
      role="dialog"
      aria-modal="true"
      aria-labelledby="optional-groups-title"
    >
      <div
        ref={cardRef}
        className="w-full max-w-md rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 id="optional-groups-title" className="text-lg font-semibold text-neutral-50">
          {recipe.name} — 설치할 구성 요소 선택
        </h3>
        <p className="mt-1 text-xs text-neutral-500">
          체크한 것만 설치됩니다. 나중에 언제든 이 화면을 다시 열어 바꿀 수 있습니다 —
          껐다 켜면 다시 받아 복원되고, 켰다 끄면 지워집니다.
        </p>

        <div className="mt-4 max-h-[50vh] space-y-2 overflow-y-auto">
          {phase.kind === "loading" && <p className="text-sm text-neutral-400">불러오는 중...</p>}
          {phase.kind === "error" && (
            <p className="text-sm text-red-300">불러오기 실패: {phase.message}</p>
          )}
          {phase.kind === "ready" &&
            recipe.optional_groups.map((g) => (
              <label
                key={g.id}
                className="flex cursor-pointer items-start gap-2 rounded border border-neutral-800 bg-neutral-950/40 p-2.5 hover:border-neutral-700"
              >
                <input
                  type="checkbox"
                  className="mt-0.5 cursor-pointer"
                  checked={checked.has(g.id)}
                  onChange={() => toggle(g.id)}
                />
                <span className="min-w-0 block text-sm text-neutral-200">{g.label}</span>
              </label>
            ))}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <Button variant="outline" size="sm" onClick={onCancel} className="cursor-pointer">
            취소
          </Button>
          <Button
            data-groups-confirm
            size="sm"
            disabled={phase.kind !== "ready"}
            onClick={() => onConfirm(Array.from(checked))}
            className="cursor-pointer"
          >
            확인
          </Button>
        </div>
      </div>
    </div>
    </Portal>
  );
}

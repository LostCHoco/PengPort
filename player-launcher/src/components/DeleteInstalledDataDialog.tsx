// 설치된 데이터 삭제 확인 다이얼로그 — 선택적 그룹이 있는 레시피 전용(그룹이 없으면
// `Library.tsx`가 이 다이얼로그를 안 띄우고 기존 native `confirm()`으로 바로 처리).
//
// "전체 삭제"(베이스 포함 전부)와 "선택한 그룹만 삭제"(체크한 그룹만, 베이스+다른
// 그룹은 유지) 두 경로를 한 다이얼로그에서 제공 — 설치 쪽은 선택 다이얼로그에서
// 그룹을 끄면 이미 그 그룹만 부분 삭제되는데, "삭제" 메뉴만 항상 전체 삭제라
// 비대칭이었던 걸 맞춘다.
//
// `OptionalGroupsDialog.tsx`와 같은 손수-만든 modal 패턴.

import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";
import type { Recipe } from "@/lib/library";

interface Props {
  /** null 이면 닫힘. */
  recipe: Recipe | null;
  onConfirmAll: () => void;
  onConfirmGroups: (groups: string[]) => void;
  onCancel: () => void;
}

export function DeleteInstalledDataDialog({ recipe, onConfirmAll, onConfirmGroups, onCancel }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (recipe) setChecked(new Set());
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
    if (recipe) cardRef.current?.querySelector<HTMLButtonElement>("[data-delete-cancel]")?.focus();
  }, [recipe]);

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
      aria-labelledby="delete-installed-title"
    >
      <div
        ref={cardRef}
        className="w-full max-w-md rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 id="delete-installed-title" className="text-lg font-semibold text-neutral-50">
          {recipe.name} — 설치된 데이터 삭제
        </h3>
        <p className="mt-1 text-xs text-neutral-500">
          지운 데이터는 되돌릴 수 없습니다. 라이브러리 목록에는 그대로 남아 나중에 다시
          설치할 수 있습니다.
        </p>

        <div className="mt-4 space-y-2">
          {recipe.optional_groups.map((g) => (
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

        <div className="mt-6 flex flex-wrap justify-end gap-2">
          <Button data-delete-cancel variant="outline" size="sm" onClick={onCancel} className="cursor-pointer">
            취소
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={checked.size === 0}
            onClick={() => onConfirmGroups(Array.from(checked))}
            className="cursor-pointer border-red-900/60 text-red-300 hover:bg-red-950/50"
          >
            선택한 구성 요소만 삭제 ({checked.size})
          </Button>
          <Button
            size="sm"
            onClick={onConfirmAll}
            className="cursor-pointer bg-red-900 text-red-100 hover:bg-red-800"
          >
            전체 삭제
          </Button>
        </div>
      </div>
    </div>
    </Portal>
  );
}

// 설치/업데이트 시 override 파일 드리프트 충돌 확인 다이얼로그 — `library_install`이
// `InstallOutcome.has_override_conflicts`를 반환했을 때 뜬다.
//
// `Literal` override는 전체 덮어쓰기라, 레시피 선언값이 바뀐 뒤 재조정을 돌리면
// 그 사이 사용자가 앱을 실행하며 직접 바꾼 값(예: 인게임 그래픽 설정)이 그대로
// 사라질 수 있다. 백엔드가 "PengPort가 마지막으로 실제로 쓴 내용"과 지금 디스크
// 내용이 다른 파일만 골라 여기로 넘긴다 — 파일마다 3가지 중 하나를 고른다:
// 무시하고 업데이트 / 업데이트하지 않기(다음에 또 안 물어봄) / 디스크의 지금 내용을
// 레시피에 반영(로컬 사본만).
//
// `OptionalGroupsDialog.tsx`와 같은 손수-만든 modal 패턴.

import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";
import { Select } from "@/components/ui/form-fields";
import { useDraggablePosition } from "@/lib/use-draggable-position";
import type { OverrideConflict, OverrideConflictResolution, RecipeSummary } from "@/lib/library";

type Choice = "overwrite" | "skip" | "adopt_disk";

const CHOICE_LABELS: Record<Choice, string> = {
  overwrite: "무시하고 업데이트",
  skip: "업데이트하지 않기",
  adopt_disk: "지금 파일을 레시피에 반영",
};

interface Props {
  /** null 이면 닫힘. */
  recipe: RecipeSummary | null;
  conflicts: OverrideConflict[];
  onConfirm: (resolutions: OverrideConflictResolution[]) => void;
  onCancel: () => void;
}

export function OverrideConflictDialog({ recipe, conflicts, onConfirm, onCancel }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);
  // 안전한 쪽("업데이트하지 않기")을 기본값으로 — 실수로 사용자가 직접 바꾼 값을
  // 덮어쓰는 것보다, 한 번 더 확인시키는 쪽이 낫다는 판단.
  const [choices, setChoices] = useState<Map<string, Choice>>(new Map());
  const { style: dragStyle, onHeaderMouseDown } = useDraggablePosition(recipe !== null);

  useEffect(() => {
    if (!recipe) return;
    setChoices(new Map(conflicts.map((c) => [c.path, "skip" as Choice])));
  }, [recipe, conflicts]);

  useEffect(() => {
    if (!recipe) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [recipe, onCancel]);

  useEffect(() => {
    if (recipe) cardRef.current?.querySelector<HTMLButtonElement>("[data-conflict-confirm]")?.focus();
  }, [recipe]);

  if (!recipe) return null;

  const setChoice = (path: string, choice: Choice) => {
    setChoices((prev) => new Map(prev).set(path, choice));
  };

  const handleConfirm = () => {
    const resolutions: OverrideConflictResolution[] = conflicts.map((c) => ({
      action: choices.get(c.path) ?? "skip",
      path: c.path,
    }));
    onConfirm(resolutions);
  };

  return (
    <Portal>
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="override-conflict-title"
    >
      <div
        ref={cardRef}
        className="w-full max-w-lg rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
        style={dragStyle}
        onClick={(e) => e.stopPropagation()}
      >
        <h3
          id="override-conflict-title"
          className="text-lg font-semibold text-neutral-50"
          onMouseDown={onHeaderMouseDown}
        >
          {recipe.name} — 설치된 파일이 변경됨
        </h3>
        <p className="mt-1 text-xs text-neutral-500">
          아래 파일들은 레시피 내용이 바뀌었는데, 마지막으로 설치했을 때와 디스크의
          지금 내용이 다릅니다 — 그 사이 직접 수정했을 수 있습니다. 파일마다 어떻게
          처리할지 고르세요.
        </p>

        <div className="mt-4 max-h-[50vh] space-y-2 overflow-y-auto">
          {conflicts.map((c) => (
            <div
              key={c.path}
              className="space-y-1.5 rounded border border-neutral-800 bg-neutral-950/40 p-2.5"
            >
              <p className="break-all font-mono text-xs text-neutral-300">{c.path}</p>
              <Select
                value={choices.get(c.path) ?? "skip"}
                onChange={(v) => setChoice(c.path, v as Choice)}
                options={(Object.keys(CHOICE_LABELS) as Choice[]).map((k) => ({
                  value: k,
                  label: CHOICE_LABELS[k],
                }))}
              />
            </div>
          ))}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <Button variant="outline" size="sm" onClick={onCancel} className="cursor-pointer">
            취소
          </Button>
          <Button
            data-conflict-confirm
            size="sm"
            onClick={handleConfirm}
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

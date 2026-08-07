// `.pengz` 파일 임포트 confirm 다이얼로그. 옛 InviteDialog 대체.
//
// 트러스트 모델(app_library_essence.md): 파일을 열 때 포함된 항목 전체를 한 번에
// 보여주고 딱 한 번만 confirm — 항목별 반복 confirm 없음. 이게 지나면 실행은 항상
// 완전 원클릭(재질문 없음). 옛 InviteDialog 처럼 "운영자 preview"(Tier 1) 를 fetch 하는
// 게 아니라, 번들 자체(임포트될 레시피 목록)를 미리보기 한다 — 인스턴스라는 신뢰
// 대상이 없어졌으니 신뢰 대상도 "이 파일이 뭘 담고 있는가"로 바뀐 것.

import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";
import { libraryPreviewImportFile } from "@/lib/library";
import type { ImportPreview } from "@/lib/library";
import { useDraggablePosition } from "@/lib/use-draggable-position";

export interface ImportRequest {
  path: string;
}

interface Props {
  request: ImportRequest | null;
  onAccept: () => void;
  onDecline: () => void;
  processing?: boolean;
}

type PreviewState =
  | { status: "loading" }
  | { status: "ok"; preview: ImportPreview }
  | { status: "error"; message: string };

export function ImportDialog({ request, onAccept, onDecline, processing = false }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);
  const [preview, setPreview] = useState<PreviewState>({ status: "loading" });
  const { style: dragStyle, onHeaderMouseDown } = useDraggablePosition(request !== null);

  useEffect(() => {
    if (!request) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !processing) onDecline();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [request, processing, onDecline]);

  useEffect(() => {
    if (!request) return;
    cardRef.current?.querySelector<HTMLButtonElement>("[data-import-decline]")?.focus();
  }, [request]);

  useEffect(() => {
    if (!request) return;
    let cancelled = false;
    setPreview({ status: "loading" });
    libraryPreviewImportFile(request.path)
      .then((p) => {
        if (!cancelled) setPreview({ status: "ok", preview: p });
      })
      .catch((e) => {
        if (!cancelled) setPreview({ status: "error", message: String(e) });
      });
    return () => {
      cancelled = true;
    };
  }, [request]);

  if (!request) return null;

  const okToConfirm =
    preview.status === "ok" &&
    (preview.preview.items.length > 0 || preview.preview.third_party_apps.length > 0);

  return (
    <Portal>
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="import-title"
    >
      <div
        ref={cardRef}
        className="w-full max-w-md rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
        style={dragStyle}
        onClick={(e) => e.stopPropagation()}
      >
        <h3
          id="import-title"
          className="text-lg font-semibold text-neutral-50"
          onMouseDown={onHeaderMouseDown}
        >
          라이브러리에 추가
        </h3>

        <div className="mt-3 space-y-3 text-sm text-neutral-300">
          <p>이 파일에 포함된 앱을 라이브러리에 추가할까요?</p>

          <ImportItemsPreview preview={preview} />

          <p className="text-xs text-amber-200/80">
            아는 사람이 보낸 파일인지 확인하세요. 모르는 출처는 거부하세요.
          </p>
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <Button
            data-import-decline
            variant="outline"
            size="sm"
            disabled={processing}
            onClick={onDecline}
            className="cursor-pointer"
          >
            거부
          </Button>
          <Button
            size="sm"
            disabled={processing || !okToConfirm}
            onClick={onAccept}
            className="min-w-[80px] cursor-pointer"
          >
            {processing ? "추가 중..." : "추가"}
          </Button>
        </div>
      </div>
    </div>
    </Portal>
  );
}

function ImportItemsPreview({ preview }: { preview: PreviewState }) {
  const isEmpty =
    preview.status === "ok" &&
    preview.preview.items.length === 0 &&
    preview.preview.third_party_apps.length === 0;

  return (
    <div className="rounded border border-neutral-800 bg-neutral-950/60 p-3 text-xs">
      {preview.status === "loading" && (
        <div className="text-neutral-400">파일 내용 확인 중…</div>
      )}

      {isEmpty && <div className="text-amber-300">⚠ 빈 파일입니다.</div>}

      {preview.status === "ok" && preview.preview.items.length > 0 && (
        <ul className="space-y-1.5">
          {preview.preview.items.map((item) => (
            <li key={item.id} className="flex items-center gap-2">
              {item.icon_url && (
                <img
                  src={item.icon_url}
                  alt=""
                  className="h-5 w-5 shrink-0 rounded bg-neutral-800 object-cover"
                  onError={(e) => ((e.target as HTMLImageElement).style.display = "none")}
                />
              )}
              <span className="min-w-0 flex-1 truncate text-neutral-200">{item.name}</span>
              {item.already_in_library && (
                <span className="shrink-0 rounded bg-neutral-800 px-1.5 py-0.5 text-[10px] text-neutral-400">
                  갱신됨
                </span>
              )}
            </li>
          ))}
        </ul>
      )}

      {preview.status === "ok" && preview.preview.third_party_apps.length > 0 && (
        <ul className="mt-2 space-y-1.5 border-t border-neutral-800 pt-2">
          {preview.preview.third_party_apps.map((app) => (
            <li key={app.id} className="flex items-center gap-2">
              <span className="min-w-0 flex-1 truncate text-neutral-400">
                {app.label} <span className="text-neutral-600">(실행 도구)</span>
              </span>
              {app.already_registered && (
                <span className="shrink-0 rounded bg-neutral-800 px-1.5 py-0.5 text-[10px] text-neutral-400">
                  갱신됨
                </span>
              )}
            </li>
          ))}
        </ul>
      )}

      {preview.status === "error" && (
        <div className="space-y-1">
          <div className="text-amber-300">⚠ 파일을 확인하지 못했습니다.</div>
          <div className="break-all font-mono text-[11px] text-neutral-400">
            {preview.message}
          </div>
        </div>
      )}
    </div>
  );
}

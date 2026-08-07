// 앱 내부 스타일의 확인/알림 팝업 — native `window.confirm()`/`alert()`나
// `@tauri-apps/plugin-dialog`의 OS 다이얼로그 대신 쓴다. 이유 둘: (1) native
// `window.confirm()`은 이 Tauri(WebView2) 환경에서 대화상자를 안 띄우고 조용히
// 통과해버리는 버그가 있었음(확인 없이 파괴적 동작이 그대로 실행됨) (2) OS 다이얼로그는
// 플러그인 버전으로 그 버그는 고쳤지만, 앱의 나머지 다크 테마 UI와 안 어울리는
// Windows 기본 팝업이라 일관성이 깨짐.
//
// `await confirm(...)`/`await message(...)`(플러그인)와 똑같은 호출부 형태를 유지하도록
// Promise 기반 훅으로 만든다 — 각 호출부는 상태 관리를 직접 안 하고 그대로
// `const ok = await confirmAsync(...)` 형태를 씀. 컴포넌트 하나당 자기 훅 인스턴스를
// 쓰므로(전역 Context 불필요 — 카드 수가 많지 않아 인스턴스 비용 무시 가능) `dialog`를
// 그 컴포넌트의 JSX 안 아무 데나 렌더링하면 된다(Portal이라 실제 위치는 무관).

import { useCallback, useState } from "react";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";

export type DialogKind = "info" | "warning" | "error";

interface PendingDialog {
  message: string;
  kind: DialogKind;
  /** "confirm"은 취소/확인 두 버튼, "message"는 확인 버튼 하나(정보/에러 알림용). */
  mode: "confirm" | "message";
  resolve: (ok: boolean) => void;
}

const KIND_BUTTON_CLASS: Record<DialogKind, string> = {
  info: "",
  warning: "bg-red-700 hover:bg-red-600",
  error: "bg-red-700 hover:bg-red-600",
};

export function useConfirmDialog() {
  const [pending, setPending] = useState<PendingDialog | null>(null);

  /** `@tauri-apps/plugin-dialog`의 `confirm(message, {kind})`와 같은 시그니처 —
   * 취소/확인 두 버튼, 확인 시 true. */
  const confirmAsync = useCallback((message: string, kind: DialogKind = "info"): Promise<boolean> => {
    return new Promise((resolve) => {
      setPending({ message, kind, mode: "confirm", resolve });
    });
  }, []);

  /** `@tauri-apps/plugin-dialog`의 `message(text, {kind})`와 같은 용도 — 확인 버튼
   * 하나뿐인 정보/에러 알림. */
  const messageAsync = useCallback((message: string, kind: DialogKind = "info"): Promise<void> => {
    return new Promise((resolve) => {
      setPending({ message, kind, mode: "message", resolve: () => resolve() });
    });
  }, []);

  const handleConfirm = () => {
    pending?.resolve(true);
    setPending(null);
  };
  const handleCancel = () => {
    pending?.resolve(false);
    setPending(null);
  };

  const dialog = pending && (
    <Portal>
      <div
        className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
        role="dialog"
        aria-modal="true"
      >
        <div
          className="w-full max-w-md rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
          onClick={(e) => e.stopPropagation()}
        >
          <p className="whitespace-pre-line text-sm text-neutral-200">{pending.message}</p>
          <div className="mt-6 flex justify-end gap-2">
            {pending.mode === "confirm" && (
              <Button variant="outline" size="sm" onClick={handleCancel} className="cursor-pointer">
                취소
              </Button>
            )}
            <Button
              size="sm"
              onClick={handleConfirm}
              className={`min-w-[64px] cursor-pointer ${KIND_BUTTON_CLASS[pending.kind]}`}
            >
              확인
            </Button>
          </div>
        </div>
      </div>
    </Portal>
  );

  return { confirmAsync, messageAsync, dialog };
}

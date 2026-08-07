import { useCallback, useEffect, useState } from "react";
import { NavLink, Outlet, useNavigate } from "react-router";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { checkForUpdate } from "@/lib/updater";
import {
  UpdatePromptDialog,
  type UpdatePromptInfo,
} from "@/components/UpdatePromptDialog";
import { ImportDialog, type ImportRequest } from "@/components/ImportDialog";
import { ModeSelectorDialog } from "@/components/ModeSelectorDialog";
import { libraryConfirmImportFile, takePendingPengzFile } from "@/lib/library";
import { useMode } from "@/lib/mode-context";
import { performWipe } from "@/lib/wipe";
import { uninstallSelf } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { useConfirmDialog } from "@/components/ui/confirm-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";

export default function App() {
  const [version, setVersion] = useState<string>("");
  const [updatePrompt, setUpdatePrompt] = useState<UpdatePromptInfo | null>(null);
  const [importRequest, setImportRequest] = useState<ImportRequest | null>(null);
  const [importProcessing, setImportProcessing] = useState(false);
  const { messageAsync, dialog: confirmDialog } = useConfirmDialog();
  // reloadKey — 임포트 성공 시 bump. Library 페이지가 Outlet context 로 받아 강제 refetch.
  const [reloadKey, setReloadKey] = useState(0);
  const { mode, setMode } = useMode();
  const [ephemeralExit, setEphemeralExit] = useState<
    | { kind: "asking" }
    | { kind: "wiping" }
    | { kind: "error"; message: string }
    | null
  >(null);

  useEffect(() => {
    if (mode !== "ephemeral") return;
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    (async () => {
      unlisten = await win.onCloseRequested((event) => {
        event.preventDefault();
        setEphemeralExit((cur) => cur ?? { kind: "asking" });
      });
    })();
    return () => {
      unlisten?.();
    };
  }, [mode]);

  const onEphemeralExitCancel = useCallback(() => {
    setEphemeralExit(null);
  }, []);

  const onEphemeralExitConfirm = useCallback(async () => {
    setEphemeralExit({ kind: "wiping" });
    try {
      await performWipe();
      await uninstallSelf();
    } catch (e) {
      setEphemeralExit({ kind: "error", message: String(e) });
    }
  }, []);

  const navigate = useNavigate();

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(""));
  }, []);

  useEffect(() => {
    (async () => {
      try {
        const info = await checkForUpdate();
        if (info.available && info.install && info.version) {
          setUpdatePrompt({
            version: info.version,
            currentVersion: info.currentVersion,
            body: info.body,
            install: info.install,
          });
        }
      } catch (e) {
        console.warn("[updater] check failed:", e);
      }
    })();
  }, []);

  // ====== `.pengz` 파일(더블클릭) 처리 ======
  //
  // 콜드 스타트(이 process 가 파일 경로를 인자로 받으며 새로 뜬 경우)는 frontend 가
  // mount 되기 전에 이벤트가 지나갈 수 있어 `takePendingPengzFile` 로 1회 조회
  // (`lib.rs`가 상태로 잡아둠). 핫 스타트(이미 실행 중일 때 더블클릭)는 single_instance
  // 콜백이 직접 emit 하는 `"pengz-file-opened"` 이벤트로 옴 — 그 시점엔 frontend 가
  // 이미 떠 있어 레이스 없음.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const pending = await takePendingPengzFile();
        if (pending) setImportRequest({ path: pending });
      } catch (e) {
        console.warn("[pengz] 콜드 스타트 파일 조회 실패:", e);
      }
      try {
        unlisten = await listen<string>("pengz-file-opened", (e) => {
          setImportRequest({ path: e.payload });
        });
      } catch (e) {
        console.warn("[pengz] 이벤트 리스너 등록 실패:", e);
      }
    })();
    return () => {
      unlisten?.();
    };
  }, []);

  const handleImportAccept = useCallback(async () => {
    if (!importRequest) return;
    setImportProcessing(true);
    try {
      await libraryConfirmImportFile(importRequest.path);
      setReloadKey((k) => k + 1);
      setImportRequest(null);
      navigate("/");
    } catch (e) {
      console.error("[import] 임포트 실패:", e);
      await messageAsync(`추가 실패: ${e}`, "error");
    } finally {
      setImportProcessing(false);
    }
  }, [importRequest, navigate, messageAsync]);

  const handleImportDecline = useCallback(() => {
    setImportRequest(null);
  }, []);

  return (
    <div className="flex h-full bg-neutral-950 text-neutral-100">
      <aside className="flex w-56 shrink-0 flex-col border-r border-neutral-800 bg-neutral-900/50">
        <div className="border-b border-neutral-800 px-5 py-4">
          <h1 className="text-lg font-semibold tracking-tight">PengPort</h1>
        </div>
        <nav className="flex flex-1 flex-col gap-1 p-3">
          <SidebarLink to="/">라이브러리</SidebarLink>
          <SidebarLink to="/third-party">서드파티 앱</SidebarLink>
          <SidebarLink to="/settings">설정</SidebarLink>
        </nav>
        {(version || mode === "ephemeral") && (
          <div className="border-t border-neutral-800 px-5 py-3 text-[11px]">
            {mode === "ephemeral" && (
              <div className="mb-1 inline-flex items-center gap-1 rounded bg-amber-900/40 px-1.5 py-0.5 text-amber-200">
                <span className="h-1.5 w-1.5 rounded-full bg-amber-400" aria-hidden />
                <span>1회용 모드</span>
              </div>
            )}
            {version && <div className="text-neutral-500">v{version}</div>}
          </div>
        )}
      </aside>
      <main className="flex-1 overflow-y-auto">
        <Outlet context={reloadKey} />
      </main>

      <UpdatePromptDialog
        info={updatePrompt}
        onDismiss={() => setUpdatePrompt(null)}
      />
      <ImportDialog
        request={importRequest}
        onAccept={handleImportAccept}
        onDecline={handleImportDecline}
        processing={importProcessing}
      />
      {mode === null && <ModeSelectorDialog onSelect={setMode} />}
      <EphemeralExitDialog
        state={ephemeralExit}
        onCancel={onEphemeralExitCancel}
        onConfirm={onEphemeralExitConfirm}
      />
      {confirmDialog}
    </div>
  );
}

function EphemeralExitDialog({
  state,
  onCancel,
  onConfirm,
}: {
  state:
    | { kind: "asking" }
    | { kind: "wiping" }
    | { kind: "error"; message: string }
    | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  if (!state) return null;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      role="dialog"
      aria-modal="true"
    >
      <div className="w-full max-w-md rounded-lg border border-amber-900/60 bg-neutral-900 p-6 shadow-2xl">
        <h3 className="text-lg font-semibold text-amber-200">
          1회용 모드 종료
        </h3>
        <div className="mt-3 space-y-2 text-sm text-neutral-300">
          <p>
            PengPort 를 종료하면 이 PC 의 모든 데이터 (라이브러리 / third-party 앱 계정 /
            실행 데이터 / 캐시) 와 PengPort 자체가{" "}
            <span className="font-medium text-red-300">자동 정리</span>됩니다.
          </p>
          <p className="text-xs text-neutral-500">
            다른 PC 에서 다시 사용하려면 PengPort 를 재설치하세요.
          </p>
          {state.kind === "wiping" && (
            <p className="text-xs text-amber-200">
              정리 중... PengPort 가 곧 닫힙니다.
            </p>
          )}
          {state.kind === "error" && (
            <p className="text-xs text-red-300" title={state.message}>
              실패: {state.message.length > 200 ? state.message.slice(0, 200) + "..." : state.message}
            </p>
          )}
        </div>
        <div className="mt-6 flex justify-end gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={onCancel}
            disabled={state.kind === "wiping"}
            className="cursor-pointer"
          >
            취소 (계속 사용)
          </Button>
          <Button
            size="sm"
            onClick={onConfirm}
            disabled={state.kind === "wiping"}
            className="cursor-pointer bg-red-700 hover:bg-red-600"
          >
            종료 + 정리
          </Button>
        </div>
      </div>
    </div>
  );
}

function SidebarLink({
  to,
  children,
}: {
  to: string;
  children: React.ReactNode;
}) {
  return (
    <NavLink
      to={to}
      end
      className={({ isActive }) =>
        [
          "rounded-md px-3 py-2 text-sm transition-colors",
          isActive
            ? "bg-neutral-800 text-neutral-50"
            : "text-neutral-400 hover:bg-neutral-800/50 hover:text-neutral-100",
        ].join(" ")
      }
    >
      {children}
    </NavLink>
  );
}

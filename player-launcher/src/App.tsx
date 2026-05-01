import { useCallback, useEffect, useRef, useState } from "react";
import { NavLink, Outlet, useNavigate } from "react-router";
import { getVersion } from "@tauri-apps/api/app";
import { getCurrent, onOpenUrl } from "@tauri-apps/plugin-deep-link";
import { checkForUpdate } from "@/lib/updater";
import { useInstances } from "@/lib/instances-context";
import {
  UpdatePromptDialog,
  type UpdatePromptInfo,
} from "@/components/UpdatePromptDialog";
import { InviteDialog, type InviteRequest } from "@/components/InviteDialog";
import { ModeSelectorDialog } from "@/components/ModeSelectorDialog";
import { instanceToken } from "@/lib/secrets";
import { useMode } from "@/lib/mode-context";
import { performWipe } from "@/lib/wipe";
import { uninstallSelf } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { getCurrentWindow } from "@tauri-apps/api/window";

const LS_LIBRARY_EXPANDED = "pengport.sidebar.library_expanded";

export default function App() {
  const [version, setVersion] = useState<string>("");
  const [updatePrompt, setUpdatePrompt] = useState<UpdatePromptInfo | null>(null);
  const [invite, setInvite] = useState<InviteRequest | null>(null);
  const [inviteProcessing, setInviteProcessing] = useState(false);
  // mode 상태는 context. Settings 도 같은 source 공유 (mode 변경 UI 가 거기 있음).
  const { mode, setMode } = useMode();
  // ephemeral 종료 confirm dialog. close 시 mode==='ephemeral' 이면 표시.
  const [ephemeralExit, setEphemeralExit] = useState<
    | { kind: "asking" }
    | { kind: "wiping" }
    | { kind: "error"; message: string }
    | null
  >(null);

  // ephemeral 모드 시 window close 가로채서 confirm dialog 표시.
  // 일반 모드면 normal close — listener 등록 안 함.
  useEffect(() => {
    if (mode !== "ephemeral") return;
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    (async () => {
      unlisten = await win.onCloseRequested((event) => {
        // 진행 중 (wiping) 이면 중복 dialog 방지 — close 그냥 무시.
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
      // uninstall_self silent — NSIS uninstaller /S flag + background user data cleanup +
      // 자체 process exit. 이 함수 호출 후 PengPort process 가 곧 종료됨.
      await uninstallSelf({ silent: true });
      // 위 호출 후 곧 exit 되니 이 줄은 도달 안 함.
    } catch (e) {
      setEphemeralExit({ kind: "error", message: String(e) });
    }
  }, []);
  const { instances, activeId, active, setActive, add, refreshActiveCatalog } =
    useInstances();
  const navigate = useNavigate();

  // useInstances() 의 instances/add 는 closure 안에서 stale 위험 — onOpenUrl 콜백은 한 번 등록
  // 후 hot launch 마다 호출되므로 ref 로 항상 최신 value 접근.
  const instancesRef = useRef(instances);
  instancesRef.current = instances;
  const addRef = useRef(add);
  addRef.current = add;
  const setActiveRef = useRef(setActive);
  setActiveRef.current = setActive;
  const refreshActiveCatalogRef = useRef(refreshActiveCatalog);
  refreshActiveCatalogRef.current = refreshActiveCatalog;
  // "라이브러리" 그룹은 헤더 클릭으로 접었다 펼 수 있는 collapsible. 페이지 이동은 안 하고
  // 하위 인스턴스 항목 클릭이 navigation 트리거. 사용자 선호는 localStorage 에 저장.
  const [libraryExpanded, setLibraryExpanded] = useState<boolean>(() => {
    const v = localStorage.getItem(LS_LIBRARY_EXPANDED);
    return v === null ? true : v === "true";
  });
  const toggleLibrary = () => {
    setLibraryExpanded((v) => {
      const next = !v;
      localStorage.setItem(LS_LIBRARY_EXPANDED, String(next));
      return next;
    });
  };

  // 사이드바 표시용 현재 앱 버전. Tauri 가 native 에서 가져옴 (tauri.conf.json 의 version).
  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(""));
  }, []);

  // 앱 기동 시 업데이트 체크 — 새 버전 있으면 dialog 띄워 사용자 동의 받음.
  // silent auto-install 안 함 (0.1.3 부터 정책 변경): 사용자 모르는 사이 재시작이 어색.
  // "다음에" 누르면 그 세션 동안만 닫힘 — 다음 launch 에 다시 묻음.
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

  // ====== Deep link (`pengport://join?...`) 처리 ======
  //
  // 두 진입 경로:
  //  - cold start: OS 가 PengPort 를 새로 띄우면서 argv 로 URL 전달 → getCurrent() 가 반환
  //  - hot: 이미 실행 중인데 link 클릭 → single_instance 가 첫 인스턴스로 forward,
  //    deep_link plugin 의 onOpenUrl 이 emit
  //
  // 둘 다 같은 핸들러로 dispatch → InviteDialog.

  const handleDeepLinkUrls = useCallback((urls: string[] | null) => {
    if (!urls || urls.length === 0) return;
    // 한 번에 여러 URL 이 들어와도 첫 번째만 처리 (드문 케이스 — 사용자는 하나만 클릭).
    const parsed = parseInviteUrl(urls[0]);
    if (!parsed) {
      console.warn("[deep-link] 알 수 없는 URL:", urls[0]);
      return;
    }
    const exists = instancesRef.current.some((i) => i.url === parsed.url);
    setInvite({ url: parsed.url, token: parsed.token, alreadyExists: exists });
  }, []);

  // mount 시 cold-start URL 확인 + onOpenUrl listener 등록.
  // listener 는 unmount 시 cleanup — App.tsx 는 단일 root 라 effective lifetime = app lifetime.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    (async () => {
      try {
        const current = await getCurrent();
        handleDeepLinkUrls(current);
      } catch (e) {
        console.warn("[deep-link] getCurrent 실패:", e);
      }
      try {
        unlisten = await onOpenUrl(handleDeepLinkUrls);
      } catch (e) {
        console.warn("[deep-link] onOpenUrl 등록 실패:", e);
      }
    })();
    return () => {
      unlisten?.();
    };
  }, [handleDeepLinkUrls]);

  const handleInviteAccept = useCallback(async () => {
    if (!invite) return;
    setInviteProcessing(true);
    try {
      // 같은 URL 의 entry 가 이미 있으면 add() 가 그걸 재사용 + setActive 까지 함.
      // alreadyExists 든 아니든 add() 로 단일 흐름.
      const entry = addRef.current({ url: invite.url });
      if (invite.token.length > 0) {
        await instanceToken.save(entry.id, invite.token);
      }
      setActiveRef.current(entry.id);
      // alreadyExists 케이스: active id 가 안 변하므로 PspLibrary 의 useEffect 가
      // trigger 안 됨 — 화면이 옛 데이터/옛 토큰으로 stale. 명시 reload 로 새 토큰 fetch 강제.
      // 새 가입 케이스도 호출해도 무해 (id 변경으로 이미 trigger + reloadKey 도 trigger 면
      // React batching 으로 1번 실행).
      refreshActiveCatalogRef.current();
      setInvite(null);
      navigate("/");
    } catch (e) {
      console.error("[deep-link] 가입 실패:", e);
      // 실패 케이스 — dialog 는 닫지 않고 사용자에게 다시 시도 기회. 단순화 위해 alert.
      alert(`가입 실패: ${e}`);
    } finally {
      setInviteProcessing(false);
    }
  }, [invite, navigate]);

  const handleInviteDecline = useCallback(() => {
    setInvite(null);
  }, []);

  return (
    <div className="flex h-full bg-neutral-950 text-neutral-100">
      <aside className="flex w-56 shrink-0 flex-col border-r border-neutral-800 bg-neutral-900/50">
        <div className="border-b border-neutral-800 px-5 py-4">
          <h1 className="text-lg font-semibold tracking-tight">PengPort</h1>
          <p className="text-xs text-neutral-400">
            {active?.name ?? active?.url ?? "통합 런처"}
          </p>
        </div>
        <nav className="flex flex-1 flex-col gap-1 p-3">
          {/* 라이브러리 그룹 헤더 — 클릭 시 인스턴스 list 접기/펼치기. 페이지 이동 없음. */}
          <button
            type="button"
            onClick={toggleLibrary}
            aria-expanded={libraryExpanded}
            className="flex cursor-pointer items-center gap-2 rounded-md px-3 py-2 text-left text-sm text-neutral-400 transition-colors hover:bg-neutral-800/50 hover:text-neutral-100"
          >
            <Caret expanded={libraryExpanded} />
            <span>라이브러리</span>
          </button>

          {libraryExpanded && (
            <div className="mt-1 ml-2 flex flex-col gap-0.5">
              {instances.map((inst) => {
                const isActive = inst.id === activeId;
                const label = inst.name ?? inst.url;
                return (
                  <button
                    key={inst.id}
                    type="button"
                    onClick={() => {
                      navigate("/");
                      if (inst.id !== activeId) setActive(inst.id);
                    }}
                    className={[
                      "flex cursor-pointer items-center gap-2 truncate rounded-md px-2.5 py-1.5 text-left text-xs transition-colors active:scale-[0.98]",
                      isActive
                        ? "bg-neutral-800/70 text-neutral-100 hover:bg-neutral-700/70"
                        : "text-neutral-500 hover:bg-neutral-800/40 hover:text-neutral-300",
                    ].join(" ")}
                    title={inst.url}
                  >
                    <span
                      className={[
                        "inline-block h-1.5 w-1.5 shrink-0 rounded-full",
                        isActive ? "bg-emerald-400" : "bg-neutral-600",
                      ].join(" ")}
                      aria-hidden
                    />
                    <span className="truncate">{label}</span>
                  </button>
                );
              })}
              <button
                type="button"
                onClick={() => {
                  navigate("/");
                  if (activeId !== null) setActive(null);
                }}
                className="flex cursor-pointer items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-xs text-neutral-500 transition-colors hover:bg-neutral-800/40 hover:text-neutral-300 active:scale-[0.98]"
              >
                <span className="inline-block w-1.5" aria-hidden />
                <span>+ 인스턴스 추가</span>
              </button>
            </div>
          )}

          <div className="mt-3 flex flex-col gap-1">
            <SidebarLink to="/third-party">서드파티 앱</SidebarLink>
            <SidebarLink to="/settings">설정</SidebarLink>
          </div>
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
        <Outlet />
      </main>

      <UpdatePromptDialog
        info={updatePrompt}
        onDismiss={() => setUpdatePrompt(null)}
      />
      <InviteDialog
        request={invite}
        onAccept={handleInviteAccept}
        onDecline={handleInviteDecline}
        processing={inviteProcessing}
      />
      {mode === null && <ModeSelectorDialog onSelect={setMode} />}
      <EphemeralExitDialog
        state={ephemeralExit}
        onCancel={onEphemeralExitCancel}
        onConfirm={onEphemeralExitConfirm}
      />
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
            PengPort 를 종료하면 이 PC 의 모든 데이터 (인스턴스 / 토큰 / Prism 계정 /
            Minecraft 세이브 / 캐시) 와 PengPort 자체가{" "}
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
            className="cursor-pointer bg-red-700 hover:bg-red-600 disabled:cursor-not-allowed"
          >
            종료 + 정리
          </Button>
        </div>
      </div>
    </div>
  );
}

/**
 * 초대 링크 파싱: `pengport://join?url=<encoded>&token=<encoded>`.
 *
 * - host 부분 (`join`) 이 action selector. 향후 다른 action 추가 가능 (예: presence).
 * - url / token 은 percent-encoded — searchParams 가 자동 decode.
 * - url 은 https/http 만 허용 (다른 scheme 차단). token 은 빈 값도 허용 (auth.type=none 인 인스턴스 대응).
 *
 * 검증 실패 시 null — 호출자가 무시.
 */
function parseInviteUrl(raw: string): { url: string; token: string } | null {
  let parsed: URL;
  try {
    parsed = new URL(raw);
  } catch {
    return null;
  }
  if (parsed.protocol !== "pengport:") return null;
  // host 또는 pathname 으로 action 식별. WHATWG URL 이 custom scheme 에서 host 가 비어있고
  // pathname 에 `//join` 이 들어가는 경우도 있어 둘 다 검사.
  const action = parsed.host || parsed.pathname.replace(/^\/+/, "");
  if (action !== "join") return null;

  const target = parsed.searchParams.get("url");
  const token = parsed.searchParams.get("token") ?? "";
  if (!target) return null;
  try {
    const t = new URL(target);
    if (t.protocol !== "https:" && t.protocol !== "http:") return null;
    // 끝 trailing slash 정규화 — `https://x.com` 과 `https://x.com/` 가 같은 인스턴스로
    // 인식되어야 함 (instances.ts 의 URL 비교는 string 정확 일치). 대부분 사용자 입력이
    // trailing slash 없는 형태라 그쪽으로 통일.
    const normalized = t.origin + (t.pathname === "/" ? "" : t.pathname);
    return { url: normalized, token: token.trim() };
  } catch {
    return null;
  }
}

function Caret({ expanded }: { expanded: boolean }) {
  return (
    <svg
      width="10"
      height="10"
      viewBox="0 0 10 10"
      fill="currentColor"
      aria-hidden
      className={`shrink-0 transition-transform ${expanded ? "rotate-90" : ""}`}
    >
      <path d="M3 1l4 4-4 4z" />
    </svg>
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

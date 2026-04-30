import { useEffect, useState } from "react";
import { NavLink, Outlet, useNavigate } from "react-router";
import { getVersion } from "@tauri-apps/api/app";
import { checkForUpdate } from "@/lib/updater";
import { useInstances } from "@/lib/instances-context";
import {
  UpdatePromptDialog,
  type UpdatePromptInfo,
} from "@/components/UpdatePromptDialog";

const LS_LIBRARY_EXPANDED = "pengport.sidebar.library_expanded";

export default function App() {
  const [version, setVersion] = useState<string>("");
  const [updatePrompt, setUpdatePrompt] = useState<UpdatePromptInfo | null>(null);
  const { instances, activeId, active, setActive } = useInstances();
  const navigate = useNavigate();
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
        {version && (
          <div className="border-t border-neutral-800 px-5 py-3 text-[11px] text-neutral-500">
            v{version}
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
    </div>
  );
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

import { useEffect, useState } from "react";
import { NavLink, Outlet, useNavigate } from "react-router";
import { getVersion } from "@tauri-apps/api/app";
import { checkForUpdate } from "@/lib/updater";
import { useInstances } from "@/lib/instances-context";

export default function App() {
  const [version, setVersion] = useState<string>("");
  const { instances, activeId, active, setActive } = useInstances();
  const navigate = useNavigate();

  // 사이드바 표시용 현재 앱 버전. Tauri 가 native 에서 가져옴 (tauri.conf.json 의 version).
  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(""));
  }, []);

  // 앱 기동 시 업데이트 자동 적용.
  // 새 버전 있으면 silent 로 다운로드 + minisign 서명 검증 + 설치 + 자동 재기동.
  // Prism / 게임은 별도 process 라 PengPort 재기동에 영향 없음.
  // 실패는 console.warn 만 (사용자 방해 없이 다음 기동 시 재시도).
  useEffect(() => {
    (async () => {
      try {
        const info = await checkForUpdate();
        if (info.available && info.install) {
          console.info(
            `[updater] auto-installing ${info.version} (current ${info.currentVersion})`,
          );
          await info.install();
        }
      } catch (e) {
        console.warn("[updater] auto-update failed:", e);
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
          <SidebarLink to="/">라이브러리</SidebarLink>

          {/* 인스턴스 list (라이브러리 sub-item) */}
          <div className="mt-1 ml-2 flex flex-col gap-0.5">
            {instances.map((inst) => {
              const isActive = inst.id === activeId;
              const label = inst.name ?? inst.url;
              return (
                <button
                  key={inst.id}
                  type="button"
                  onClick={() => {
                    setActive(inst.id);
                    navigate("/");
                  }}
                  className={[
                    "flex items-center gap-2 truncate rounded-md px-2.5 py-1.5 text-left text-xs transition-colors",
                    isActive
                      ? "bg-neutral-800/70 text-neutral-100"
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
                setActive(null);
                navigate("/");
              }}
              className="flex items-center gap-2 rounded-md px-2.5 py-1.5 text-left text-xs text-neutral-500 transition-colors hover:bg-neutral-800/40 hover:text-neutral-300"
            >
              <span className="inline-block w-1.5" aria-hidden />
              <span>+ 인스턴스 추가</span>
            </button>
          </div>

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

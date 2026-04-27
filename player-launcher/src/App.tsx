import { useEffect, useState } from "react";
import { NavLink, Outlet } from "react-router";
import { getVersion } from "@tauri-apps/api/app";
import { checkForUpdate, getUpdateTokenSource } from "@/lib/updater";

export default function App() {
  const [version, setVersion] = useState<string>("");

  // 사이드바 표시용 현재 앱 버전. Tauri 가 native 에서 가져옴 (tauri.conf.json 의 version).
  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(""));
  }, []);

  // 앱 기동 시 업데이트 자동 적용.
  // - 토큰이 아예 없으면 (= OOBE 미완료) skip → 라이브러리의 TokenSetup 카드가 처리.
  // - 토큰 있으면 silent 로 다운로드 + 서명 검증 + 설치 + 자동 재기동.
  //   Prism / 게임은 별도 process 라 PengPort 재기동에 영향 없음.
  // - 실패는 console.warn 만 (사용자 방해 없이 다음 기동 시 재시도).
  useEffect(() => {
    (async () => {
      try {
        if ((await getUpdateTokenSource()) === "none") return;
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
          <p className="text-xs text-neutral-400">펭돌서버 통합 런처</p>
        </div>
        <nav className="flex flex-1 flex-col gap-1 p-3">
          <SidebarLink to="/">라이브러리</SidebarLink>
          <SidebarLink to="/settings">설정</SidebarLink>
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

function SidebarLink({ to, children }: { to: string; children: React.ReactNode }) {
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

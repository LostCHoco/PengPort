// 서드파티 앱 관리 페이지.
//
// PSP 정신상 third-party app (PrismLauncher, 미래의 Jellyfin/Nextcloud client 등) 의
// detect / download / configure 흐름을 한 곳에서. service 와 분리.
//
// 등록된 third-party app 들의 list — 현재는 PrismLauncher 만. 새 app 추가는
// shared/src/actions/third_party/<app>.rs + 여기 list 에 카드 추가.

import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  detectPrism,
  downloadPrism,
  removeBundledPrism,
  setPrismOverride,
  type PrismLocation,
  type PrismSource,
} from "@/lib/api";

export default function ThirdPartyApps() {
  return (
    <div className="space-y-6 p-8">
      <header>
        <h2 className="text-2xl font-semibold">서드파티 앱</h2>
        <p className="mt-1 text-xs text-neutral-500">
          인스턴스 service 가 사용하는 외부 앱 (런처, 미디어 클라이언트 등). 한
          곳에서 detect / 자동 다운로드 / 폴더 지정.
        </p>
      </header>

      <PrismCard />
    </div>
  );
}

function prismSourceLabel(s: PrismSource): string {
  switch (s) {
    case "user":
      return "사용자 지정";
    case "env":
      return "환경변수 (개발)";
    case "portable":
      return "exe 옆 폴더";
    case "bundled":
      return "전용 (자동 다운로드)";
    case "system":
      return "시스템 설치본";
    case "path":
      return "PATH 발견";
  }
}

type PrismOp =
  | { kind: "idle" }
  | { kind: "downloading" }
  | { kind: "removing" }
  | { kind: "error"; message: string };

function PrismCard() {
  const [location, setLocation] = useState<PrismLocation | null | undefined>(
    undefined,
  );
  const [op, setOp] = useState<PrismOp>({ kind: "idle" });

  const refresh = async () => {
    try {
      setLocation(await detectPrism());
    } catch (e) {
      setLocation(null);
      setOp({ kind: "error", message: String(e) });
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const onPickFolder = async () => {
    setOp({ kind: "idle" });
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        directory: true,
        multiple: false,
        title: "prismlauncher.exe 가 있는 폴더 선택",
      });
      if (!picked || typeof picked !== "string") return;
      const next = await setPrismOverride(picked);
      setLocation(next);
    } catch (e) {
      setOp({ kind: "error", message: String(e) });
    }
  };

  const onClearOverride = async () => {
    setOp({ kind: "idle" });
    try {
      const next = await setPrismOverride("");
      setLocation(next);
    } catch (e) {
      setOp({ kind: "error", message: String(e) });
    }
  };

  const onDownload = async () => {
    setOp({ kind: "downloading" });
    try {
      await downloadPrism();
      await refresh();
      setOp({ kind: "idle" });
    } catch (e) {
      setOp({ kind: "error", message: String(e) });
    }
  };

  const onRemoveBundled = async () => {
    setOp({ kind: "removing" });
    try {
      const next = await removeBundledPrism();
      setLocation(next);
      setOp({ kind: "idle" });
    } catch (e) {
      setOp({ kind: "error", message: String(e) });
    }
  };

  if (location === undefined) {
    return (
      <section className="space-y-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5">
        <h3 className="text-sm font-medium text-neutral-200">PrismLauncher</h3>
        <p className="text-xs text-neutral-400">탐지 중...</p>
      </section>
    );
  }

  return (
    <section className="space-y-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5">
      <div className="flex items-baseline justify-between">
        <div>
          <h3 className="text-sm font-medium text-neutral-200">
            PrismLauncher
          </h3>
          <p className="mt-0.5 text-[11px] text-neutral-500">
            Minecraft 인스턴스 관리 도구
          </p>
        </div>
        {location && (
          <span className="text-[11px] text-neutral-500">
            출처: {prismSourceLabel(location.source)}
          </span>
        )}
      </div>

      {location ? (
        <>
          <div className="space-y-1 text-xs text-neutral-400">
            <div>
              실행 파일:{" "}
              <code className="text-neutral-300">{location.exe}</code>
            </div>
            <div>
              데이터 폴더:{" "}
              <code className="text-neutral-300">{location.data_dir}</code>
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" variant="outline" onClick={onPickFolder}>
              다른 폴더 지정
            </Button>
            {location.source === "user" && (
              <Button size="sm" variant="outline" onClick={onClearOverride}>
                지정 해제 (자동 탐색 복귀)
              </Button>
            )}
            {location.source === "bundled" && (
              <Button
                size="sm"
                variant="outline"
                onClick={onRemoveBundled}
                disabled={op.kind === "removing"}
              >
                {op.kind === "removing" ? "삭제 중..." : "전용 Prism 삭제"}
              </Button>
            )}
          </div>
        </>
      ) : (
        <>
          <p className="text-xs text-neutral-400">
            PrismLauncher 를 찾을 수 없습니다. 자동 다운로드하거나 폴더를 직접
            지정하세요.
          </p>
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              onClick={onDownload}
              disabled={op.kind === "downloading"}
            >
              {op.kind === "downloading" ? "다운로드 중..." : "자동 다운로드"}
            </Button>
            <Button size="sm" variant="outline" onClick={onPickFolder}>
              폴더 직접 지정
            </Button>
          </div>
        </>
      )}

      {op.kind === "error" && (
        <p className="text-xs text-red-300" title={op.message}>
          실패:{" "}
          {op.message.length > 200
            ? op.message.slice(0, 200) + "..."
            : op.message}
        </p>
      )}
    </section>
  );
}

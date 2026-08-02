// 앱 라이브러리 카드 — 옛 ServiceCard 대체.
//
// 옛 버전과의 핵심 차이: manifest.endpoints.status/events 를 원격으로 폴링/구독하던
// 코드를 전부 제거했다. Recipe 에는 애초에 그런 엔드포인트가 없다 — 최소 상태(설치됨/
// 실행중)는 로컬 프로세스 기준으로만 판단한다(app_library_essence.md). 메트릭/배지/
// presence 도 마찬가지로 없음 — "실행 이후 그 앱 내부는 PengPort 책임 아님".
//
// 메뉴의 "라이브러리에서 제거"와 "삭제"는 서로 반대 축이다 — "제거"는 목록에서만
// 빼고 설치된 파일(게임 세이브 등)은 그대로 둔다, "삭제"는 반대로 설치된 파일만
// 지우고 라이브러리 항목은 남긴다(나중에 다시 [설치] 누르면 재설치 가능). 옛
// ServiceCard 의 "앱 제거"는 이 둘이 하나로 묶여있었는데, 카탈로그가 없어진 지금은
// "목록에서 빼기"와 "데이터 지우기"가 별개 의도라 나눴다.
//
// 실행 클릭은 이 카드가 직접 invoke 하지 않고 onLaunch 콜백으로 부모(Library.tsx)에
// 위임 — outcome 분기(설치 dialog 오픈 등)는 부모가 소유.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";
import { useConfirmDialog } from "@/components/ui/confirm-dialog";
import { InstallDiffDialog } from "@/components/InstallDiffDialog";
import { isServiceRunning, stopServer } from "@/lib/api";
import { libraryInstallStatus, libraryStageManualArchiveFile } from "@/lib/library";
import type { ArtifactVerification, InstallStatus, Recipe } from "@/lib/library";

// `commands/library.rs::reconcile_install` 이 emit 하는 `install:*` 이벤트 페이로드.
// 백엔드는 `serde_json::json!`(타입 없이 camelCase 키)로 직접 만들어 보내므로 프론트
// 쪽도 (`server:started` 등 기존 prism 이벤트와 같은 컨벤션으로) 여기 인라인 타입만
// 둔다 — 별도 command 가 아니라 새 필드 추가에 스키마 동기화 부담이 없음.
interface StepStartedPayload {
  recipeId: string;
  index: number;
  total: number;
  kind: "archive" | "file";
  label: string;
}
interface DownloadProgressPayload {
  recipeId: string;
  label: string;
  downloadedBytes: number;
  totalBytes: number | null;
}
interface ExtractProgressPayload {
  recipeId: string;
  label: string;
  extractedEntries: number;
  totalEntries: number;
}
/** 직접 다운로드가 안 돼(응답이 실제 파일이 아니라 페이지로 판명) 기본 브라우저를
 * 열어준 시점에 한 번만 emit. 바이트 진행률이 없는 대신(사람이 직접 받는 중이라
 * PengPort가 모름) 대기 상태 UI + "직접 파일 선택" 폴백을 보여줄 근거로 쓴다.
 * `verification`은 "직접 파일 선택" 시 그대로 되돌려 보내 그 자리에서 검증하는 데
 * 쓴다(`handleSelectManualFile` 참고). */
interface BrowserDownloadWaitingPayload {
  recipeId: string;
  url: string;
  verification: ArtifactVerification;
}

interface InstallProgress {
  index: number;
  total: number;
  label: string;
  phase: "downloading" | "extracting" | "applying" | "waiting_for_browser";
  downloadedBytes?: number;
  totalBytes?: number | null;
  extractedEntries?: number;
  totalEntries?: number;
  /** 최근 `download-progress` 이벤트 간격 기준 순간 전송 속도(EMA 로 완만하게)
   * — 백엔드가 이벤트를 안 주므로 프론트가 연속 이벤트의 바이트/시간 델타로 계산. */
  speedBytesPerSec?: number;
  /** `waiting_for_browser` 단계에서만 채워짐 — "직접 파일 선택"이 즉시 검증할 때 씀. */
  verification?: ArtifactVerification;
}

/** 압축 URL/오버라이드 경로 전체를 보여주기엔 너무 기니 마지막 경로 조각만. */
function shortLabel(label: string): string {
  const parts = label.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? label;
}

function formatBytes(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}GB`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(0)}MB`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}KB`;
  return `${n}B`;
}

function formatSpeed(bytesPerSec: number): string {
  return `${formatBytes(bytesPerSec)}/s`;
}

/** `installing` 중 진행 바 아래 표시할 한 줄 — 다운로드는 total_bytes 유무에 따라
 * %/받은 양 중 아는 만큼만, 압축 해제는 항상 엔트리 개수 기준(바이트 단위는 라이브러리가
 * 엔트리별 압축 전 크기를 안 줘서 못 함 — `docs/design/INSTALL_PROGRESS.md`). */
function progressLine(p: InstallProgress): string {
  const step = `(${p.index}/${p.total})`;
  if (p.phase === "downloading") {
    if (p.downloadedBytes == null) return `준비 중 ${step} ${p.label}`;
    const speedSuffix = p.speedBytesPerSec != null ? `, ${formatSpeed(p.speedBytesPerSec)}` : "";
    if (p.totalBytes) {
      const pct = Math.min(100, Math.round((p.downloadedBytes / p.totalBytes) * 100));
      return `다운로드 중 ${step} ${p.label} — ${pct}% (${formatBytes(p.downloadedBytes)}/${formatBytes(p.totalBytes)}${speedSuffix})`;
    }
    return `다운로드 중 ${step} ${p.label} — ${formatBytes(p.downloadedBytes)} 받음${speedSuffix ? ` (${formatSpeed(p.speedBytesPerSec!)})` : ""}`;
  }
  if (p.phase === "extracting") {
    if (p.extractedEntries == null || p.totalEntries == null) return `압축 해제 중 ${step} ${p.label}`;
    return `압축 해제 중 ${step} ${p.label} — ${p.extractedEntries}/${p.totalEntries}`;
  }
  if (p.phase === "waiting_for_browser") {
    return `브라우저에서 다운로드를 완료해주세요 ${step}`;
  }
  return `적용 중 ${step} ${p.label}`;
}

/** 진행 바 폭(%) — 다운로드/압축 해제 둘 다 아는 만큼(total 정보가 있을 때)만
 * 계산, 모르면 `null`(호출자가 불확정 표시로 대체). */
function progressPercent(p: InstallProgress): number | null {
  if (p.phase === "downloading" && p.downloadedBytes != null && p.totalBytes) {
    return Math.min(100, Math.round((p.downloadedBytes / p.totalBytes) * 100));
  }
  if (p.phase === "extracting" && p.extractedEntries != null && p.totalEntries) {
    return Math.min(100, Math.round((p.extractedEntries / p.totalEntries) * 100));
  }
  return null;
}

interface Props {
  recipe: Recipe;
  /** "설치" 버튼 — 처음 설치든 이미 설치된 레시피의 변경분 반영("업데이트")이든
   * 지금 레시피와 다른 부분만 적용하는 같은 동작이라 버튼도 콜백도 하나뿐이다.
   * 설치 상태가 실제 레시피와 다르면 뱃지("업데이트 필요")로 알려준다. */
  onInstall: (recipe: Recipe) => void;
  installing: boolean;
  /** 설치 진행 중에만 진행 바 옆에 "취소" 버튼으로 노출. */
  onCancelInstall?: (recipe: Recipe) => void;
  onLaunch: (recipe: Recipe) => void;
  launching: boolean;
  onRemove?: () => Promise<void>;
  /** 설치된 데이터(대상 폴더 + 마커)만 삭제 — 라이브러리 항목은 남는다. `onRemove`와
   * 반대 축(모듈 설명 참고). 확인(native confirm 또는 부분 삭제 다이얼로그)은 호출자
   * 책임이라 이 콜백 자체는 동기(void) — `CardMenu`는 내부에서 더 확인하지 않는다. */
  onDelete?: () => void;
  /** 설치된 데이터를 지우고 처음부터 다시 설치 — `onDelete` + `onInstall`을 순서대로
   * 묶은 것과 같은 결과지만, 사용자에게는 "고장났을 때 다시 깨끗하게 깔기" 한 동작으로
   * 노출한다. 확인은 `onDelete`와 동일하게 호출자 책임. */
  onReinstall?: () => void;
  onOpenFolder?: () => Promise<void>;
  /** 이미 다른 위치에 설치된 앱을 그 폴더로 "연결"만 하는 다이얼로그를 연다 —
   * PengPort 가 직접 설치하지 않고 그 경로를 그대로 씀(`LocalRootOverrideDialog`). */
  onLinkFolder?: () => void;
  onEdit?: () => void;
  /** `.pengz` 파일로 내보내기(딥링크의 파일 버전 — 32KB 안팎 OS 커맨드라인 한도를
   * 파일 공유로 피함). 저장 위치 선택 다이얼로그를 여는 것부터 호출자 책임. */
  onExport?: () => void;
  /** 설치/업데이트/실행 중 하나가 끝날 때마다 부모가 값을 바꿔서, 카드가 설치 상태
   * 뱃지를 다시 조회하게 만든다(직접 폴링하지 않음 — 상태가 바뀔 만한 시점에만). */
  statusRefreshKey?: number;
  /** 여러 앱 선택 모드(`Library.tsx`의 "선택" 버튼) — 켜지면 카드 좌상단에 체크박스가
   * 나타난다. 설치/실행/메뉴 등 기존 상호작용은 그대로 유지(선택 모드가 다른 동작을
   * 막지 않음 — 순수 부가 기능). */
  selectionMode?: boolean;
  selected?: boolean;
  onToggleSelect?: () => void;
}

export function AppCard({
  recipe,
  onInstall,
  installing,
  onCancelInstall,
  onLaunch,
  launching,
  onRemove,
  onDelete,
  onReinstall,
  onOpenFolder,
  onLinkFolder,
  onEdit,
  onExport,
  statusRefreshKey,
  selectionMode,
  selected,
  onToggleSelect,
}: Props) {
  // third-party app 실행이면 "준비 중"→"실행 중" 단계 추적 대상 — app_id 는 안 가림
  // (Prism 전용이던 이벤트/커맨드가 전부 범용으로 바뀌어서, 등록된 어떤 앱이든 이
  // 단계 추적을 받는다. `docs/design/THIRD_PARTY_PLATFORM_MODEL.md` 참고).
  const isThirdPartyAppLaunch = recipe.launch.kind === "third_party_app_launch";
  // v5 부터는 "설치 필수" 불변식 덕에 모든 launch 종류가 로컬 폴더를 갖는다
  // (`commands/library.rs::library_open_folder`) — 아직 안 만들어졌으면 명령 자체가
  // 에러 메시지로 알려준다.
  const hasLocalFolder = true;
  const [menuOpen, setMenuOpen] = useState(false);
  const [phase, setPhase] = useState<"idle" | "preparing" | "running">("idle");
  const [installStatus, setInstallStatus] = useState<InstallStatus | null>(null);
  const [stopping, setStopping] = useState(false);
  const [showDiff, setShowDiff] = useState(false);
  const [installProgress, setInstallProgress] = useState<InstallProgress | null>(null);
  const [stagingManualFile, setStagingManualFile] = useState(false);
  const [manualFileError, setManualFileError] = useState<string | null>(null);

  const running = phase === "running" || phase === "preparing";

  // 로컬 프로세스 상태 — third-party app 실행 레시피만. 다른 launch 종류는 "실행 중"
  // 개념이 없음(open_url 은 그냥 브라우저를 열 뿐, PengPort 가 추적할 프로세스가 없다).
  //
  // `third_party_app:child_ready` 는 descriptor 가 `readiness_signal` 을 선언한 앱만
  // emit(Prism 은 declares — 모드팩 다운로드+Mojang 인증 끝나고 게임 창이 실제로 뜬
  // 시점). 선언 안 한 앱은 이 이벤트가 안 와서 "준비 중"에 계속 머무를 수 있음 —
  // 지금은 Prism 하나뿐이라 문제 없지만, `readiness_signal` 없는 앱을 추가하면 별도
  // 처리(예: server:started 직후 바로 running 취급)가 필요해질 수 있다는 걸 인지.
  useEffect(() => {
    if (!isThirdPartyAppLaunch) return;
    let cancelled = false;
    (async () => {
      try {
        const r = await isServiceRunning(recipe.id);
        if (!cancelled && r) setPhase("preparing");
      } catch {
        // ignore — 미지원 환경
      }
    })();

    const unlistens: Array<() => void> = [];
    (async () => {
      try {
        const u1 = await listen<{ serverId: string }>("server:started", (e) => {
          if (e.payload.serverId === recipe.id) setPhase("preparing");
        });
        const u2 = await listen<{ recipeId: string }>("third_party_app:child_ready", (e) => {
          if (e.payload.recipeId === recipe.id) setPhase("running");
        });
        const u3 = await listen<{ serverId: string }>("server:stopped", (e) => {
          if (e.payload.serverId === recipe.id) {
            setPhase("idle");
            setStopping(false);
          }
        });
        if (cancelled) {
          u1();
          u2();
          u3();
        } else {
          unlistens.push(u1, u2, u3);
        }
      } catch {
        // ignore
      }
    })();

    return () => {
      cancelled = true;
      unlistens.forEach((u) => u());
    };
  }, [isThirdPartyAppLaunch, recipe.id]);

  // 설치/업데이트 필요 여부 — 모든 launch 종류에 공통(더 이상 Prism 전용이 아님).
  // `statusRefreshKey` 가 바뀔 때(설치/업데이트/실행 시도 직후)와 레시피 자체가
  // 바뀔 때(편집 저장 등) 다시 조회.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const status = await libraryInstallStatus(recipe);
        if (!cancelled) setInstallStatus(status);
      } catch {
        if (!cancelled) setInstallStatus(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [recipe, statusRefreshKey]);

  // 설치/업데이트 진행률 — `installing` 인 동안만 `install:*` 을 구독(상시 리스너 아님).
  // 백엔드(`commands/library.rs::reconcile_install`)가 압축/오버라이드 항목마다
  // step-started → (download-progress/extract-progress 다수) → step-completed 순으로
  // emit — 스텝 인덱스만 알면 전체 진행 중 몇 번째인지, 세부 페이로드로 지금 다운로드
  // 중인지 압축 해제 중인지 알 수 있다.
  useEffect(() => {
    if (!installing) {
      setInstallProgress(null);
      return;
    }
    let cancelled = false;
    const unlistens: Array<() => void> = [];
    // 직전 `download-progress` 이벤트의 (바이트, 시각) — 연속 이벤트 간 델타로 속도를
    // 계산하는 데만 쓰는 로컬 상태라 state 승격 불필요(리렌더 유발 안 해도 됨).
    let lastSample: { bytes: number; time: number } | null = null;
    (async () => {
      try {
        const us = await Promise.all([
          listen<StepStartedPayload>("install:step-started", (e) => {
            if (e.payload.recipeId !== recipe.id) return;
            lastSample = null;
            setInstallProgress({
              index: e.payload.index,
              total: e.payload.total,
              label: shortLabel(e.payload.label),
              phase: e.payload.kind === "file" ? "applying" : "downloading",
            });
          }),
          listen<DownloadProgressPayload>("install:download-progress", (e) => {
            if (e.payload.recipeId !== recipe.id) return;
            const now = performance.now();
            let instantSpeed: number | undefined;
            if (lastSample) {
              const deltaBytes = e.payload.downloadedBytes - lastSample.bytes;
              const deltaSeconds = (now - lastSample.time) / 1000;
              if (deltaSeconds > 0 && deltaBytes >= 0) {
                instantSpeed = deltaBytes / deltaSeconds;
              }
            }
            lastSample = { bytes: e.payload.downloadedBytes, time: now };
            setInstallProgress((prev) => {
              if (!prev) return prev;
              // 순간 속도는 청크 타이밍에 따라 들쭉날쭉하므로 EMA 로 완만하게(브라우저
              // 다운로드 표시줄과 같은 흔한 패턴) — 새 샘플이 없으면 이전 값 유지.
              const speedBytesPerSec =
                instantSpeed == null
                  ? prev.speedBytesPerSec
                  : prev.speedBytesPerSec != null
                    ? prev.speedBytesPerSec * 0.7 + instantSpeed * 0.3
                    : instantSpeed;
              return {
                ...prev,
                phase: "downloading",
                downloadedBytes: e.payload.downloadedBytes,
                totalBytes: e.payload.totalBytes,
                speedBytesPerSec,
              };
            });
          }),
          listen<ExtractProgressPayload>("install:extract-progress", (e) => {
            if (e.payload.recipeId !== recipe.id) return;
            setInstallProgress((prev) =>
              prev
                ? { ...prev, phase: "extracting", extractedEntries: e.payload.extractedEntries, totalEntries: e.payload.totalEntries }
                : prev,
            );
          }),
          listen<BrowserDownloadWaitingPayload>("install:browser-download-waiting", (e) => {
            if (e.payload.recipeId !== recipe.id) return;
            setInstallProgress((prev) =>
              prev ? { ...prev, phase: "waiting_for_browser", verification: e.payload.verification } : prev,
            );
          }),
        ]);
        if (cancelled) {
          us.forEach((u) => u());
        } else {
          unlistens.push(...us);
        }
      } catch {
        // ignore — 미지원 환경
      }
    })();
    return () => {
      cancelled = true;
      unlistens.forEach((u) => u());
    };
  }, [installing, recipe.id]);

  /** "브라우저에서 열어서 받기" 압축이 자동으로 안 잡힐 때(다른 폴더에 저장 등) —
   * 사용자가 받은 파일을 직접 지정한다. 백엔드(`library_stage_manual_archive_file`)가
   * 이 레시피의 스크래치 "manual" 폴더에 복사한 뒤 그 자리에서 바로 검증한다 —
   * 안 맞으면 즉시 에러로 알려준다(자동 감시처럼 조용히 무시하지 않음 — 사용자가
   * 명시적으로 고른 파일이라 잘못 골랐으면 그 자리에서 알아야 함, 실사용 중 발견). */
  const handleSelectManualFile = async () => {
    if (!installProgress?.verification) return;
    setManualFileError(null);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ multiple: false, title: "다운로드받은 파일 선택" });
      if (!picked || typeof picked !== "string") return;
      setStagingManualFile(true);
      await libraryStageManualArchiveFile(recipe.id, picked, installProgress.verification);
    } catch (e) {
      setManualFileError(String(e));
    } finally {
      setStagingManualFile(false);
    }
  };

  const handleStop = async () => {
    setStopping(true);
    try {
      await stopServer(recipe.id);
    } catch (e) {
      console.warn("[AppCard] stop 실패", e);
      setStopping(false);
    }
  };

  const cardBorder =
    phase === "running"
      ? "border-emerald-500/60 ring-1 ring-emerald-500/20"
      : phase === "preparing"
        ? "border-amber-500/60 ring-1 ring-amber-500/20"
        : "border-neutral-800 hover:border-neutral-700";

  const progressPct = installing && installProgress ? progressPercent(installProgress) : null;

  // 설치/업데이트/실행 버튼은 항상 상태별로 딱 하나만 보인다 — 상태를 아직 모를 땐
  // (조회 전, `installStatus === null`) "설치 필수" 불변식에 맞춰 안전하게 미설치로
  // 취급한다(뱃지도 같은 기본값 — 로딩 중엔 아무 뱃지도 안 뜨는 것과 동일 원칙).
  const needsInstall =
    installStatus == null ||
    installStatus.kind === "not_installed" ||
    installStatus.kind === "needs_optional_group_selection";
  const updateAvailable = installStatus?.kind === "update_available";

  return (
    <div
      className={`relative isolate flex h-full flex-col gap-3 rounded-lg border bg-neutral-900/60 p-5 transition-colors ${
        selectionMode && selected ? "border-blue-500 ring-2 ring-blue-500/40" : cardBorder
      }`}
    >
      {selectionMode && (
        // 선택 모드에선 카드 전체가 클릭 대상 — 설치/실행/메뉴 버튼 등 아래 콘텐츠
        // 위에 투명 레이어를 덮어 클릭을 가로챈다(버튼마다 개별적으로 막을 필요 없이
        // 한 곳에서 처리). 선택 여부는 우상단의 작은 원(비활성, 시각 표시 전용)으로.
        <button
          type="button"
          onClick={onToggleSelect}
          aria-pressed={selected ?? false}
          aria-label={`${recipe.name} 선택`}
          className="absolute inset-0 z-10 cursor-pointer rounded-lg"
        />
      )}
      {recipe.recipe_info.background_url && (
        // `-z-10` — 카드 자신의 배경/테두리 위, 나머지(헤더·버튼 등) 일반 콘텐츠
        // 아래에 그려지게(포지션 없는 자식은 z-index auto/양수 포지션 자식보다 항상
        // 나중에 칠해짐). 부모에 `isolate`가 꼭 필요하다 — `relative`만으로는 새
        // 스태킹 컨텍스트가 안 생겨서 `-z-10`이 이 카드 안에 갇히지 않고 훨씬 바깥
        // (그리드 전체)까지 새어나가 버린다(그래서 처음엔 안 보였다). `rounded-lg`를
        // 이 div 자체에 줘서 카드 모서리를 따라 잘리게 한다 — 부모에
        // `overflow-hidden`을 걸면 "더보기" 드롭다운 메뉴까지 잘려서, 대신 이
        // 레이어 하나에만 둥근 모서리를 준다. 그라디언트로 어둡게 깔아 "은은한"
        // 느낌을 내고 텍스트 대비도 지킨다.
        <div
          className="absolute inset-0 -z-10 rounded-lg bg-cover bg-center"
          style={{
            backgroundImage: `linear-gradient(rgba(9,9,11,0.72), rgba(9,9,11,0.85)), url(${JSON.stringify(recipe.recipe_info.background_url)})`,
          }}
        />
      )}
      <div className="flex items-start justify-between gap-3">
        {/* `min-h-9`(아이콘 크기)로 아이콘 없는 카드도 아이콘 있는 카드와 같은 높이를
            예약 — 안 그러면 이름 한 줄 높이(아이콘보다 작음)만큼만 차지해서, 아이콘
            유무가 섞인 grid row 에서 카드 높이가 들쭉날쭉해 보였다. */}
        <div className="flex min-h-9 min-w-0 items-center gap-3">
          {recipe.recipe_info.icon_url && (
            <AppIcon key={recipe.recipe_info.icon_url} url={recipe.recipe_info.icon_url} />
          )}
          <div className="min-w-0">
            <h3 className="truncate text-lg font-semibold text-neutral-50">
              {recipe.name}
            </h3>
          </div>
        </div>
        {selectionMode ? (
          // "더보기" 버튼과 정확히 같은 자리(h-7 w-7 슬롯)에 선택 표시만 대신 놓는다 —
          // 서로 다른 위치 계산식(absolute 오프셋 vs flex)을 쓰면 모드 전환마다 자리가
          // 미묘하게 어긋나 보이므로, 같은 flex 슬롯을 공유하게 만든다. `relative z-20`은
          // 순수 위치 유지용(오프셋 없음)이면서, 카드 전체를 덮는 선택 클릭 레이어
          // (`absolute z-10`, 위)보다 위에 그려지게 하기 위함 — 안 그러면 배지가 그
          // 레이어에 가려 안 보인다.
          <div
            className="pointer-events-none relative z-20 flex h-7 w-7 shrink-0 items-center justify-center"
            aria-hidden
          >
            <div
              className={`flex h-5 w-5 items-center justify-center rounded-full border-2 ${
                selected
                  ? "border-blue-500 bg-blue-500 text-white"
                  : "border-neutral-600 bg-neutral-900/80"
              }`}
            >
              {selected && (
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={3}>
                  <path d="M5 13l4 4L19 7" strokeLinecap="round" strokeLinejoin="round" />
                </svg>
              )}
            </div>
          </div>
        ) : (
          (onRemove || onDelete || onReinstall || onEdit || onExport || onLinkFolder || (hasLocalFolder && onOpenFolder)) && (
            <CardMenu
              open={menuOpen}
              onOpenChange={setMenuOpen}
              // 설치/업데이트 진행 중엔 삭제/제거/재설치를 막는다 — 다운로드 중인 폴더를
              // 지우면 두 작업이 같은 파일을 동시에 건드리는 경합이 생긴다. 멈추고
              // 싶으면 아래 진행 바의 "취소" 버튼을 먼저 써야 한다.
              onRemove={installing ? undefined : onRemove}
              onDelete={installing ? undefined : onDelete}
              onReinstall={installing ? undefined : onReinstall}
              onEdit={onEdit}
              onOpenFolder={hasLocalFolder ? onOpenFolder : undefined}
              onLinkFolder={onLinkFolder}
              onExport={onExport}
              displayName={recipe.name}
            />
          )
        )}
      </div>

      {installing && installProgress && (
        <div className="flex flex-col gap-1">
          <div className="h-1 w-full overflow-hidden rounded-full bg-neutral-800">
            <div
              className={`h-full bg-blue-500 ${progressPct == null ? "w-2/5 animate-pulse" : "transition-all"}`}
              style={progressPct == null ? undefined : { width: `${progressPct}%` }}
            />
          </div>
          <div className="flex items-center justify-between gap-2">
            <p className="min-w-0 flex-1 truncate text-xs text-neutral-400" title={progressLine(installProgress)}>
              {progressLine(installProgress)}
            </p>
            {installProgress.phase === "waiting_for_browser" && (
              <button
                type="button"
                disabled={stagingManualFile}
                onClick={() => void handleSelectManualFile()}
                className="shrink-0 cursor-pointer text-xs text-neutral-500 underline-offset-2 hover:text-neutral-200 hover:underline disabled:cursor-not-allowed"
              >
                {stagingManualFile ? "확인 중..." : "직접 파일 선택"}
              </button>
            )}
            {onCancelInstall && (
              <button
                type="button"
                onClick={() => onCancelInstall(recipe)}
                className="shrink-0 cursor-pointer text-xs text-neutral-500 underline-offset-2 hover:text-red-300 hover:underline"
              >
                취소
              </button>
            )}
          </div>
          {manualFileError && (
            <p className="break-all text-[11px] text-red-300">{manualFileError}</p>
          )}
        </div>
      )}

      <div className="mt-auto flex items-end justify-between gap-2">
        {/* `min-h`로 뱃지 한 줄 높이를 항상 예약 — 안 그러면 뱃지 없는 카드는 이
            영역이 아예 접혀서, 뱃지 있는 카드와 섞인 줄(grid row)에서 그 줄 전체가
            뱃지 있는 카드 높이에 맞춰 늘어나 카드마다 높이가 들쭉날쭉해 보였다. */}
        <div className="flex min-h-6 min-w-0 flex-wrap items-center gap-1">
          {phase === "preparing" && (
            <Badge className="bg-amber-900/50 text-amber-200" title="모드팩 다운로드 / Mojang 인증 / world load 중">
              <Spinner />
              준비 중
            </Badge>
          )}
          {phase === "running" && (
            <Badge className="bg-emerald-900/50 text-emerald-200">
              <Dot className="bg-emerald-400 animate-pulse" />
              실행 중
            </Badge>
          )}
          {installStatus?.kind === "not_installed" && (
            <Badge className="bg-neutral-800 text-neutral-300" title="설치 버튼을 눌러야 실행할 수 있습니다">
              <Dot className="bg-neutral-500" />
              미설치
            </Badge>
          )}
          {installStatus?.kind === "update_available" && (
            <button
              type="button"
              onClick={() => setShowDiff(true)}
              title={`레시피와 안 맞는 스텝 ${installStatus.pending}/${installStatus.total}개 — 눌러서 어느 부분이 다른지 확인`}
              className="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-full bg-blue-900/50 px-2.5 py-0.5 text-xs font-medium text-blue-200 transition-colors hover:bg-blue-900/80"
            >
              <Dot className="bg-blue-400" />
              업데이트 필요
            </button>
          )}
        </div>
        {running ? (
          <Button
            size="sm"
            variant="outline"
            disabled={stopping}
            onClick={() => void handleStop()}
            className="min-w-[90px] cursor-pointer border-red-700/60 text-red-200 hover:bg-red-950/40 disabled:cursor-not-allowed"
          >
            {stopping ? (
              <span className="inline-flex items-center gap-1.5">
                <Spinner />
                종료 중
              </span>
            ) : (
              "종료"
            )}
          </Button>
        ) : needsInstall || updateAvailable ? (
          <Button
            size="sm"
            variant="outline"
            disabled={installing}
            onClick={() => onInstall(recipe)}
            // 카드 배경 이미지가 비치지 않도록 — 기본 outline variant는
            // 다크모드에서 `bg-input/30`(반투명)이라 배경이 있는 카드에선 버튼이
            // 배경에 비쳐 보인다.
            className="cursor-pointer border-neutral-700 bg-neutral-800 hover:bg-neutral-700 disabled:cursor-not-allowed dark:bg-neutral-800 dark:hover:bg-neutral-700"
          >
            {installing ? (
              <span className="inline-flex items-center gap-1.5">
                <Spinner />
                처리 중
              </span>
            ) : updateAvailable ? (
              "업데이트"
            ) : (
              "설치"
            )}
          </Button>
        ) : (
          <Button
            size="sm"
            disabled={launching}
            onClick={() => onLaunch(recipe)}
            className="min-w-[70px] cursor-pointer shadow-sm transition-all hover:shadow-md hover:brightness-110 hover:scale-[1.04] active:scale-[0.96] disabled:cursor-not-allowed"
          >
            {launching ? (
              <span className="inline-flex items-center gap-1.5">
                <Spinner />
                실행 중
              </span>
            ) : (
              "실행"
            )}
          </Button>
        )}
      </div>

      <InstallDiffDialog recipe={showDiff ? recipe : null} onClose={() => setShowDiff(false)} />
    </div>
  );
}

function Badge({
  children,
  className = "",
  title,
}: {
  children: React.ReactNode;
  className?: string;
  title?: string;
}) {
  return (
    <span
      title={title}
      className={`flex shrink-0 items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ${className}`}
    >
      {children}
    </span>
  );
}

/** 로드 실패 시 아이콘 자리를 조용히 숨긴다. 호출부(`AppCard`)가 `key={url}`로
 * 렌더링해서, URL을 나중에(레시피 편집으로) 고치면 이 컴포넌트가 통째로 리마운트되어
 * 실패 상태가 자동으로 풀린다. `<img>`에 `onError`로 직접 `style.display`를
 * 건드리면 React가 그 변경을 모르는 채로 DOM 노드를 재사용해서, 한 번 실패한 아이콘은
 * URL을 고쳐도 라이브러리 새로고침(리마운트) 전까진 계속 숨어 있는 버그가 있었다 —
 * 실패 여부를 React state로 관리해서 고침. */
function AppIcon({ url }: { url: string }) {
  const [failed, setFailed] = useState(false);
  if (failed) return null;
  return (
    <img
      src={url}
      alt=""
      className="h-9 w-9 shrink-0 rounded bg-neutral-800 object-cover"
      onError={() => setFailed(true)}
    />
  );
}

function Dot({ className = "" }: { className?: string }) {
  return <span className={`h-1.5 w-1.5 rounded-full ${className}`} />;
}

function Spinner() {
  return (
    <span
      className="h-3 w-3 animate-spin rounded-full border-2 border-current border-t-transparent"
      aria-label="처리 중"
    />
  );
}

function CardMenu({
  open,
  onOpenChange,
  onRemove,
  onDelete,
  onReinstall,
  onEdit,
  onOpenFolder,
  onLinkFolder,
  onExport,
  displayName,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onRemove?: () => Promise<void>;
  onDelete?: () => void;
  onReinstall?: () => void;
  onEdit?: () => void;
  onOpenFolder?: () => Promise<void>;
  onLinkFolder?: () => void;
  onExport?: () => void;
  displayName: string;
}) {
  // 트리거 버튼과 메뉴(포탈로 `document.body`에 붙음, 서로 다른 서브트리) 둘 다에
  // 대해 "바깥 클릭"을 판정해야 한다 — 메뉴가 더 이상 트리거의 DOM 자손이 아니라서.
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ top: number; right: number } | null>(null);
  const { confirmAsync, dialog: confirmDialog } = useConfirmDialog();

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      const target = e.target as Node;
      if (
        triggerRef.current &&
        !triggerRef.current.contains(target) &&
        menuRef.current &&
        !menuRef.current.contains(target)
      ) {
        onOpenChange(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onOpenChange(false);
    };
    // 스크롤/리사이즈로 트리거 위치가 바뀌면 재계산하는 대신 그냥 닫는다 — 팝오버가
    // 트리거에서 떨어진 채로 떠 있는 것보다 안전.
    const onScrollOrResize = () => onOpenChange(false);
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    window.addEventListener("scroll", onScrollOrResize, true);
    window.addEventListener("resize", onScrollOrResize);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onScrollOrResize, true);
      window.removeEventListener("resize", onScrollOrResize);
    };
  }, [open, onOpenChange]);

  // 열릴 때 트리거 버튼의 화면 좌표를 한 번 계산 — 포탈로 옮겨진 메뉴는 더 이상
  // 트리거 밑에서 `absolute`로 자동 배치되지 않으므로 `fixed` 좌표를 직접 넘겨야 함.
  useLayoutEffect(() => {
    if (!open || !triggerRef.current) {
      setPosition(null);
      return;
    }
    const rect = triggerRef.current.getBoundingClientRect();
    setPosition({ top: rect.bottom + 4, right: window.innerWidth - rect.right });
  }, [open]);

  const handleRemove = () => {
    onOpenChange(false);
    void (async () => {
      const ok = await confirmAsync(
        `${displayName} 을(를) 완전히 제거할까요?\n\n` +
          `설치된 파일들이 전부 지워지며 라이브러리 목록에서도 함께 사라집니다.`,
        "warning",
      );
      if (ok) void onRemove?.();
    })();
  };

  const handleDelete = () => {
    onOpenChange(false);
    // 확인은 호출자(`Library.tsx`)가 담당 — 선택적 그룹이 있으면 전용 다이얼로그(전체/
    // 부분 삭제 선택), 없으면 간단한 native confirm. 여기서 이중으로 확인하지 않는다.
    void onDelete?.();
  };

  const handleReinstall = () => {
    onOpenChange(false);
    // 확인은 `onDelete`와 동일하게 호출자(`Library.tsx`) 책임.
    onReinstall?.();
  };

  const handleOpenFolder = () => {
    onOpenChange(false);
    void onOpenFolder?.();
  };

  const handleLinkFolder = () => {
    onOpenChange(false);
    onLinkFolder?.();
  };

  const handleEdit = () => {
    onOpenChange(false);
    onEdit?.();
  };

  const handleExport = () => {
    onOpenChange(false);
    onExport?.();
  };

  return (
    <div className="relative">
      <button
        ref={triggerRef}
        type="button"
        aria-label="더보기"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => onOpenChange(!open)}
        className="flex h-7 w-7 cursor-pointer items-center justify-center rounded text-neutral-500 transition-colors hover:bg-neutral-800/60 hover:text-neutral-200"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
          <circle cx="5" cy="12" r="1.6" />
          <circle cx="12" cy="12" r="1.6" />
          <circle cx="19" cy="12" r="1.6" />
        </svg>
      </button>
      {open && position && (
        <Portal>
        <div
          ref={menuRef}
          role="menu"
          className="fixed z-50 w-56 overflow-hidden rounded-md border border-neutral-700 bg-neutral-900 py-1 shadow-lg"
          style={{ top: position.top, right: position.right }}
        >
          {onEdit && (
            <button
              type="button"
              role="menuitem"
              onClick={handleEdit}
              className="flex w-full cursor-pointer items-center px-3 py-1.5 text-left text-xs text-neutral-200 transition-colors hover:bg-neutral-800/60"
            >
              앱 편집
            </button>
          )}
          {onExport && (
            <button
              type="button"
              role="menuitem"
              onClick={handleExport}
              className="flex w-full cursor-pointer items-center px-3 py-1.5 text-left text-xs text-neutral-200 transition-colors hover:bg-neutral-800/60"
            >
              내보내기
            </button>
          )}
          {onOpenFolder && (
            <button
              type="button"
              role="menuitem"
              onClick={handleOpenFolder}
              className="flex w-full cursor-pointer items-center px-3 py-1.5 text-left text-xs text-neutral-200 transition-colors hover:bg-neutral-800/60"
            >
              설치 폴더 열기
            </button>
          )}
          {onLinkFolder && (
            <button
              type="button"
              role="menuitem"
              onClick={handleLinkFolder}
              className="flex w-full cursor-pointer items-center px-3 py-1.5 text-left text-xs text-neutral-200 transition-colors hover:bg-neutral-800/60"
            >
              로컬 폴더 연결
            </button>
          )}
          {onReinstall && (
            <button
              type="button"
              role="menuitem"
              onClick={handleReinstall}
              className="flex w-full cursor-pointer items-center px-3 py-1.5 text-left text-xs text-neutral-200 transition-colors hover:bg-neutral-800/60"
            >
              재설치
            </button>
          )}
          {onDelete && (
            <button
              type="button"
              role="menuitem"
              onClick={handleDelete}
              className="flex w-full cursor-pointer items-center px-3 py-1.5 text-left text-xs text-red-300 transition-colors hover:bg-red-950/50"
            >
              삭제
            </button>
          )}
          {onRemove && (
            <button
              type="button"
              role="menuitem"
              onClick={handleRemove}
              className="flex w-full cursor-pointer items-center px-3 py-1.5 text-left text-xs text-red-300 transition-colors hover:bg-red-950/50"
            >
              라이브러리에서 제거
            </button>
          )}
        </div>
        </Portal>
      )}
      {confirmDialog}
    </div>
  );
}

// 서드파티 앱 관리 페이지.
//
// third-party app(PrismLauncher, 미래의 Steam 등) 의 detect / download / configure
// 흐름을 한 곳에서. `list_third_party_apps`(로컬 파일 `%APPDATA%\PengPort\
// third_party_apps.json`, 링크 임포트로 채워짐)가 주는 목록을 그대로 순회해 카드를
// 그린다 — 새 third-party app 이 등록되면 이 페이지도 코드 변경 없이 카드가 하나 늘어난다.
//
// 탐지/override 지정/bundled 삭제/자동 다운로드 넷 다 이제 app_id 하나로 모든 앱에
// 통하는 범용 커맨드 — descriptor 의 `download_strategy` 유무를 `supports_download`로
// 노출해 다운로드 버튼 표시 여부를 결정한다(`docs/design/THIRD_PARTY_PLATFORM_MODEL.md` §3).

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { useConfirmDialog } from "@/components/ui/confirm-dialog";
import { ThirdPartyAppEditDialog } from "@/components/ThirdPartyAppEditDialog";
import {
  configureThirdPartyAppOverride,
  detectThirdPartyApp,
  downloadThirdPartyApp,
  listThirdPartyAppDescriptors,
  listThirdPartyApps,
  removeBundledThirdPartyApp,
  thirdPartyAppRemove,
  thirdPartyAppUpsert,
  type ThirdPartyAppLocation,
  type ThirdPartyAppSource,
  type ThirdPartyAppSummary,
} from "@/lib/api";
import type { ThirdPartyAppDescriptor } from "@/lib/library";

export default function ThirdPartyApps() {
  const [apps, setApps] = useState<ThirdPartyAppSummary[] | null>(null);
  // 편집 다이얼로그가 기존 값을 채우려면 요약(label/supports_download)이 아니라
  // 전체 descriptor 가 필요 — 카드 목록과 별개로 들고 있다가 "편집" 클릭 시 조회.
  const [descriptors, setDescriptors] = useState<ThirdPartyAppDescriptor[] | null>(null);
  // null = 닫힘, { descriptor: null } = 신규 등록, { descriptor: 값 } = 기존 편집.
  const [editing, setEditing] = useState<{ descriptor: ThirdPartyAppDescriptor | null } | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const { confirmAsync, dialog: confirmDialog } = useConfirmDialog();

  const refresh = useCallback(async () => {
    const [summaries, full] = await Promise.all([listThirdPartyApps(), listThirdPartyAppDescriptors()]);
    setApps(summaries);
    setDescriptors(full);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const handleSave = async (descriptor: ThirdPartyAppDescriptor) => {
    await thirdPartyAppUpsert(descriptor);
    setEditing(null);
    await refresh();
  };

  const handleDelete = async (app: ThirdPartyAppSummary) => {
    const ok = await confirmAsync(
      `${app.label} 등록을 해제할까요?\n\n` +
        `이 앱을 실행 방식으로 쓰는 레시피가 있다면 실행이 안 됩니다. 다운로드된 전용 사본이나 ` +
        `이미 설치된 인스턴스는 지워지지 않습니다.`,
      "warning",
    );
    if (!ok) return;
    try {
      await thirdPartyAppRemove(app.id);
      await refresh();
    } catch (e) {
      setSaveError(String(e));
    }
  };

  return (
    <div className="space-y-6 p-8">
      <header className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-2xl font-semibold">서드파티 앱</h2>
          <p className="mt-1 text-xs text-neutral-500">
            인스턴스 service 가 사용하는 외부 앱 (런처, 미디어 클라이언트 등). 한
            곳에서 detect / 자동 다운로드 / 폴더 지정.
          </p>
        </div>
        <Button size="sm" onClick={() => setEditing({ descriptor: null })} className="shrink-0 cursor-pointer">
          + 새 서드파티 앱 등록
        </Button>
      </header>

      {saveError && (
        <p className="break-all rounded border border-red-900/50 bg-red-950/30 p-2 text-xs text-red-200">
          {saveError}
        </p>
      )}

      {apps === null ? (
        <p className="text-xs text-neutral-500">불러오는 중...</p>
      ) : (
        apps.map((app) => (
          <ThirdPartyAppCard
            key={app.id}
            app={app}
            onEdit={() => {
              const full = descriptors?.find((d) => d.id === app.id);
              if (full) setEditing({ descriptor: full });
            }}
            onDelete={() => void handleDelete(app)}
          />
        ))
      )}

      {editing && (
        <ThirdPartyAppEditDialog
          descriptor={editing.descriptor}
          existingIds={apps?.map((a) => a.id) ?? []}
          onSave={handleSave}
          onCancel={() => setEditing(null)}
        />
      )}

      {confirmDialog}
    </div>
  );
}

function sourceLabel(s: ThirdPartyAppSource): string {
  switch (s) {
    case "user_override":
      return "사용자 지정";
    case "bundled":
      return "전용 (자동 다운로드)";
    case "system":
      return "시스템 설치본";
  }
}

type AppOp =
  | { kind: "idle" }
  | { kind: "downloading" }
  | { kind: "removing" }
  | { kind: "error"; message: string };

function ThirdPartyAppCard({
  app,
  onEdit,
  onDelete,
}: {
  app: ThirdPartyAppSummary;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const [location, setLocation] = useState<ThirdPartyAppLocation | null | undefined>(
    undefined,
  );
  const [op, setOp] = useState<AppOp>({ kind: "idle" });
  const canDownload = app.supports_download;

  const refresh = async () => {
    try {
      setLocation(await detectThirdPartyApp(app.id));
    } catch (e) {
      setLocation(null);
      setOp({ kind: "error", message: String(e) });
    }
  };

  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [app.id]);

  const onPickFolder = async () => {
    setOp({ kind: "idle" });
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        directory: true,
        multiple: false,
        title: `${app.label} 실행 파일이 있는 폴더 선택`,
      });
      if (!picked || typeof picked !== "string") return;
      setLocation(await configureThirdPartyAppOverride(app.id, picked));
    } catch (e) {
      setOp({ kind: "error", message: String(e) });
    }
  };

  const onClearOverride = async () => {
    setOp({ kind: "idle" });
    try {
      setLocation(await configureThirdPartyAppOverride(app.id, ""));
    } catch (e) {
      setOp({ kind: "error", message: String(e) });
    }
  };

  const onDownload = async () => {
    if (!canDownload) return;
    setOp({ kind: "downloading" });
    try {
      await downloadThirdPartyApp(app.id);
      await refresh();
      setOp({ kind: "idle" });
    } catch (e) {
      setOp({ kind: "error", message: String(e) });
    }
  };

  const onRemoveBundled = async () => {
    setOp({ kind: "removing" });
    try {
      setLocation(await removeBundledThirdPartyApp(app.id));
      setOp({ kind: "idle" });
    } catch (e) {
      setOp({ kind: "error", message: String(e) });
    }
  };

  if (location === undefined) {
    return (
      <section className="space-y-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5">
        <div className="flex items-baseline justify-between">
          <h3 className="text-sm font-medium text-neutral-200">{app.label}</h3>
          <EditDeleteActions onEdit={onEdit} onDelete={onDelete} />
        </div>
        <p className="text-xs text-neutral-400">탐지 중...</p>
      </section>
    );
  }

  return (
    <section className="space-y-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5">
      <div className="flex items-baseline justify-between">
        <div>
          <h3 className="text-sm font-medium text-neutral-200">{app.label}</h3>
        </div>
        <div className="flex items-center gap-3">
          {location && (
            <span className="text-[11px] text-neutral-500">
              출처: {sourceLabel(location.source)}
            </span>
          )}
          <EditDeleteActions onEdit={onEdit} onDelete={onDelete} />
        </div>
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
              <code className="text-neutral-300">{location.data_root}</code>
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" variant="outline" onClick={onPickFolder}>
              다른 폴더 지정
            </Button>
            {location.source === "user_override" && (
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
                {op.kind === "removing" ? "삭제 중..." : `전용 ${app.label} 삭제`}
              </Button>
            )}
          </div>
        </>
      ) : (
        <>
          <p className="text-xs text-neutral-400">
            {app.label} 를 찾을 수 없습니다.{" "}
            {canDownload ? "자동 다운로드하거나 폴더를 직접 지정하세요." : "폴더를 직접 지정하세요."}
          </p>
          <div className="flex flex-wrap gap-2">
            {canDownload && (
              <Button
                size="sm"
                onClick={onDownload}
                disabled={op.kind === "downloading"}
              >
                {op.kind === "downloading" ? "다운로드 중..." : "자동 다운로드"}
              </Button>
            )}
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

/** 카드 우측 상단 "편집"/"삭제" — 자동 다운로드/폴더 지정 등 실행 관련 버튼과
 * 섞이면 등록 관리와 실행 관리가 뒤섞여 보이므로 상단 헤더로 분리. */
function EditDeleteActions({ onEdit, onDelete }: { onEdit: () => void; onDelete: () => void }) {
  return (
    <div className="flex shrink-0 items-center gap-1 text-[11px]">
      <button type="button" onClick={onEdit} className="cursor-pointer text-neutral-400 hover:text-neutral-200">
        편집
      </button>
      <button type="button" onClick={onDelete} className="cursor-pointer text-red-300 hover:text-red-200">
        삭제
      </button>
    </div>
  );
}

// 서드파티 앱 descriptor 편집 다이얼로그 — `ThirdPartyApps.tsx`(설정 화면)의
// "+ 새 서드파티 앱 등록"(신규) / 카드별 "편집"(기존 수정) 양쪽에서 연다 —
// `descriptor: null`이면 신규. `RecipeEditDialog.tsx`와 같은 패턴(Portal 모달 +
// `Field`/`TextInput`/`Select` 공용 프리미티브 + draft state + 저장 시점 검증).
//
// `id`는 편집 불가 — 다운로드 시 bundled root(`%LOCALAPPDATA%\PengPort\<id>\`)의
// 경로 컴포넌트로 쓰이므로, 바꾸면 이미 받아둔 사본과 descriptor가 어긋난다.
// 신규 등록만 직접 입력(레시피처럼 `label`에서 자동 slugify하지 않는 이유: third-party
// app id는 사람이 기억하고 사용처를 짐작하기 쉬운 값이어야 하는 경우가 많아
// — 예: `prism_launcher` — 자동 파생보다 직접 지정이 더 명확).

import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";
import { Field, RemoveButton, Select, TextInput } from "@/components/ui/form-fields";
import { DestinationPathPicker } from "@/components/ui/file-tree-picker";
import { scanFolderRelativePaths } from "@/lib/library";
import type { DownloadStrategy, ReadinessSignal, ThirdPartyAppDescriptor } from "@/lib/library";
import { useDraggablePosition } from "@/lib/use-draggable-position";

interface Props {
  /** `null`이면 신규 등록. */
  descriptor: ThirdPartyAppDescriptor | null;
  /** 신규 등록 시 id 중복 검사용. */
  existingIds: string[];
  onSave: (descriptor: ThirdPartyAppDescriptor) => Promise<void>;
  onCancel: () => void;
}

function defaultNewDescriptor(): ThirdPartyAppDescriptor {
  return {
    id: "",
    label: "",
    exe_filename: "",
    download_strategy: null,
    post_download_marker_files: [],
    instances_subfolder: null,
    system_appdata_folder_name: null,
    readiness_signal: null,
    launch_args_template: [],
  };
}

/** `pengport_shared::ids::validate_service_id`가 허용하는 `[A-Za-z0-9_-]{1,64}`와
 * 동일 — id는 자동 파생이 아니라 직접 입력이라 저장 전에 프론트에서도 미리 안내. */
function isValidId(id: string): boolean {
  return /^[A-Za-z0-9_-]{1,64}$/.test(id);
}

export function ThirdPartyAppEditDialog({ descriptor, existingIds, onSave, onCancel }: Props) {
  const isNew = descriptor === null;
  const { style: dragStyle, onHeaderMouseDown } = useDraggablePosition(true);
  const [draft, setDraft] = useState<ThirdPartyAppDescriptor>(descriptor ?? defaultNewDescriptor());
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // 이미 설치된 폴더(또는 방금 받은 압축을 풀어둔 폴더)를 한 번 스캔해두면, 아래
  // "실행 파일 이름"/"다운로드 완료 판정 파일"을 손으로 타이핑하는 대신 트리에서
  // 클릭해서 채울 수 있다 — id/exe_filename처럼 오타가 나면 조용히 탐지/실행이
  // 실패하는 값들이라 `RecipeEditDialog`의 "폴더 불러오기" 패턴을 그대로 재사용.
  const [scannedPaths, setScannedPaths] = useState<string[] | null>(null);
  const [scannedRoot, setScannedRoot] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [exeWarning, setExeWarning] = useState<string | null>(null);
  const scannedFiles = useMemo(() => (scannedPaths ?? []).map((path) => ({ path })), [scannedPaths]);

  const handleScanFolder = async () => {
    setScanError(null);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ directory: true, multiple: false, title: "설치된 폴더 또는 압축 해제 폴더 선택" });
      if (!picked || typeof picked !== "string") return;
      setScanning(true);
      const paths = await scanFolderRelativePaths(picked);
      setScannedRoot(picked);
      setScannedPaths(paths);
    } catch (e) {
      setScanError(String(e));
    } finally {
      setScanning(false);
    }
  };

  const handleSave = async () => {
    if (!isValidId(draft.id)) {
      setError("id는 영문/숫자/`_`/`-`만, 1~64자로 입력하세요.");
      return;
    }
    if (isNew && existingIds.includes(draft.id)) {
      setError("이미 등록된 id 입니다.");
      return;
    }
    if (draft.exe_filename.trim().length === 0) {
      setError("설치 폴더를 불러와서 실행 파일을 골라주세요.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await onSave(draft);
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  };

  return (
    <Portal>
      <div
        className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
        role="dialog"
        aria-modal="true"
        aria-labelledby="third-party-app-edit-title"
      >
        <div
          className="flex max-h-[85vh] w-full max-w-lg flex-col rounded-lg border border-neutral-800 bg-neutral-900 shadow-2xl"
          style={dragStyle}
          onClick={(e) => e.stopPropagation()}
        >
          <h3
            id="third-party-app-edit-title"
            className="px-6 py-3 text-base font-semibold text-neutral-50"
            onMouseDown={onHeaderMouseDown}
          >
            {isNew ? "새 서드파티 앱 등록" : "서드파티 앱 편집"}
          </h3>

          <div className="min-h-0 flex-1 space-y-4 overflow-y-auto border-t border-neutral-800 p-6">
            <Field label="id (편집 불가, 폴더명으로 사용됨)">
              <TextInput
                value={draft.id}
                placeholder="예: prism_launcher"
                disabled={!isNew}
                onChange={(id) => setDraft({ ...draft, id })}
              />
            </Field>
            <Field label="표시 이름">
              <TextInput
                value={draft.label ?? ""}
                placeholder="비우면 id를 그대로 표시"
                onChange={(v) => setDraft({ ...draft, label: v || null })}
              />
            </Field>
            <Field label="설치 폴더에서 불러오기">
              <div className="flex items-center gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={scanning}
                  onClick={() => void handleScanFolder()}
                  className="shrink-0 cursor-pointer"
                >
                  {scanning ? "불러오는 중..." : "폴더 선택"}
                </Button>
                {scannedRoot && (
                  <span className="min-w-0 truncate text-[11px] text-neutral-500" title={scannedRoot}>
                    {scannedRoot}
                  </span>
                )}
              </div>
              {scanError && <p className="mt-1 break-all text-[11px] text-red-300">{scanError}</p>}
              <p className="mt-1 text-[11px] text-neutral-500">
                이미 설치된 폴더나 방금 받은 압축을 풀어둔 폴더를 고르세요 — 아래 실행 파일 이름은
                이 트리에서 골라야만 채워집니다(오타 방지).
              </p>
            </Field>

            <Field label="실행 파일 이름">
              <div className="flex items-center gap-2">
                <div className="min-w-0 flex-1">
                  <TextInput
                    value={draft.exe_filename}
                    placeholder="위 폴더에서 골라 채우세요"
                    readOnly
                    onChange={(exe_filename) => {
                      setExeWarning(null);
                      setDraft({ ...draft, exe_filename });
                    }}
                  />
                </div>
                <DestinationPathPicker
                  files={scannedFiles}
                  mode="file"
                  emptyLabel={scannedPaths === null ? "먼저 위에서 폴더를 불러오세요." : "폴더에 파일이 없습니다."}
                  onPick={(path) => {
                    const slash = path.lastIndexOf("/");
                    const base = slash === -1 ? path : path.slice(slash + 1);
                    setExeWarning(
                      slash === -1
                        ? null
                        : `선택한 파일이 하위 폴더(${path})에 있습니다 — 실행 파일은 데이터 폴더 바로 아래 있어야 해서 파일명(${base})만 채웠습니다.`,
                    );
                    setDraft((d) => ({ ...d, exe_filename: base }));
                  }}
                />
              </div>
              {exeWarning && <p className="mt-1 text-[11px] text-amber-400">{exeWarning}</p>}
            </Field>

            <Field label="자동 다운로드">
              <DownloadStrategyFields
                strategy={draft.download_strategy ?? null}
                onChange={(download_strategy) => setDraft({ ...draft, download_strategy })}
              />
            </Field>

            <Field label="다운로드 완료 판정 파일 (상대경로)">
              <StringListEditor
                items={draft.post_download_marker_files}
                onChange={(post_download_marker_files) => setDraft({ ...draft, post_download_marker_files })}
                placeholder="예: prismlauncher.exe"
                addLabel="+ 파일 추가"
                treePicker={{
                  files: scannedFiles,
                  emptyLabel: scannedPaths === null ? "먼저 위에서 폴더를 불러오세요." : "폴더에 파일이 없습니다.",
                }}
              />
            </Field>

            <Field label="인스턴스 하위 폴더">
              <TextInput
                value={draft.instances_subfolder ?? ""}
                placeholder="비우면 데이터 루트 바로 아래(예: instances)"
                onChange={(v) => setDraft({ ...draft, instances_subfolder: v || null })}
              />
            </Field>

            <Field label="시스템 설치 시 appdata 폴더 이름">
              <TextInput
                value={draft.system_appdata_folder_name ?? ""}
                placeholder="비우면 시스템 설치본의 appdata 폴더를 안 씀"
                onChange={(v) => setDraft({ ...draft, system_appdata_folder_name: v || null })}
              />
              <p className="mt-1 text-[11px] text-neutral-500">
                이미 설치된 실행 파일 자체는 별도 설정 없이 자동으로 찾습니다 — 이 필드는 찾았을 때 그
                데이터가 어디(`%APPDATA%\이 이름\`)에 있는지만 지정합니다.
              </p>
            </Field>

            <Field label="준비 완료 신호">
              <ReadinessSignalFields
                signal={draft.readiness_signal ?? null}
                onChange={(readiness_signal) => setDraft({ ...draft, readiness_signal })}
              />
            </Field>

            <Field label="실행 인자 템플릿 (쉼표로 구분)">
              <TextInput
                value={draft.launch_args_template.join(", ")}
                onChange={(v) =>
                  setDraft({
                    ...draft,
                    launch_args_template: v
                      .split(",")
                      .map((s) => s.trim())
                      .filter((s) => s.length > 0),
                  })
                }
              />
            </Field>
          </div>

          <div className="border-t border-neutral-800 px-6 py-4">
            {error && (
              <p className="mb-3 break-all rounded border border-red-900/50 bg-red-950/30 p-2 text-xs text-red-200">
                {error}
              </p>
            )}
            <div className="flex justify-end gap-2">
              <Button variant="outline" size="sm" disabled={saving} onClick={onCancel} className="cursor-pointer">
                취소
              </Button>
              <Button size="sm" disabled={saving} onClick={() => void handleSave()} className="min-w-[80px] cursor-pointer">
                {saving ? "저장 중..." : "저장"}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </Portal>
  );
}

// ---------------------------------------------------------------------------
// 작은 리스트/유니언 편집기 — 항목 수가 적어(수 개) `RecipeEditDialog`의 가상
// 스크롤 `ListEditor`는 과함, 단순 map 렌더링으로 충분.
// ---------------------------------------------------------------------------

function StringListEditor({
  items,
  onChange,
  placeholder,
  addLabel,
  treePicker,
}: {
  items: string[];
  onChange: (items: string[]) => void;
  placeholder?: string;
  addLabel: string;
  /** 스캔해둔 폴더 트리에서 골라 추가하는 보조 버튼(선택) — 손으로 타이핑하는
   * 경로가 손쉽게 어긋나는 필드(예: 다운로드 완료 판정 파일)에만 붙인다. */
  treePicker?: { files: { path: string }[]; emptyLabel?: string };
}) {
  const [draftValue, setDraftValue] = useState("");
  const add = () => {
    const trimmed = draftValue.trim();
    if (!trimmed || items.includes(trimmed)) return;
    onChange([...items, trimmed]);
    setDraftValue("");
  };
  return (
    <div className="space-y-1">
      {items.map((item, i) => (
        <div key={i} className="flex items-center gap-1">
          <span className="min-w-0 flex-1 truncate rounded bg-neutral-950 px-1.5 py-0.5 font-mono text-[11px] text-neutral-300">
            {item}
          </span>
          <RemoveButton onClick={() => onChange(items.filter((_, j) => j !== i))} />
        </div>
      ))}
      <div className="flex gap-1">
        <TextInput value={draftValue} placeholder={placeholder} onChange={setDraftValue} />
        <Button type="button" size="sm" variant="outline" className="shrink-0 cursor-pointer" onClick={add}>
          추가
        </Button>
        {treePicker && (
          <DestinationPathPicker
            files={treePicker.files}
            mode="file"
            emptyLabel={treePicker.emptyLabel}
            onPick={(path) => {
              if (!items.includes(path)) onChange([...items, path]);
            }}
          />
        )}
      </div>
      {items.length === 0 && <p className="text-[11px] text-neutral-500">{addLabel.replace(/^\+ /, "")} 없음</p>}
    </div>
  );
}

type DownloadKind = DownloadStrategy["kind"] | "none";

const DOWNLOAD_KIND_LABELS: Record<DownloadKind, string> = {
  none: "자동 다운로드 없음 (수동 설치만)",
  static_url: "고정 URL",
  github_latest_release: "GitHub 최신 릴리스",
};

function defaultDownloadStrategy(kind: DownloadStrategy["kind"]): DownloadStrategy {
  switch (kind) {
    case "static_url":
      return { kind: "static_url", url: "", verification: { kind: "sha256", hash: "" } };
    case "github_latest_release":
      return { kind: "github_latest_release", repo: "", asset_name_pattern: "" };
  }
}

function DownloadStrategyFields({
  strategy,
  onChange,
}: {
  strategy: DownloadStrategy | null;
  onChange: (s: DownloadStrategy | null) => void;
}) {
  const kind: DownloadKind = strategy?.kind ?? "none";
  return (
    <div className="space-y-2">
      <Select
        value={kind}
        onChange={(k) => onChange(k === "none" ? null : defaultDownloadStrategy(k as DownloadStrategy["kind"]))}
        options={(Object.keys(DOWNLOAD_KIND_LABELS) as DownloadKind[]).map((k) => ({
          value: k,
          label: DOWNLOAD_KIND_LABELS[k],
        }))}
      />
      {strategy?.kind === "static_url" && (
        <div className="space-y-2 rounded border border-neutral-800 bg-neutral-950/40 p-2">
          <TextInput
            value={strategy.url}
            placeholder="다운로드 URL"
            onChange={(url) => onChange({ ...strategy, url })}
          />
          <p className="text-[11px] text-neutral-500">
            무결성 해시(SHA256)는 실제 다운로드해본 뒤 채워야 정확합니다 — 지금은 빈 값으로 저장되니
            추후 파일을 받아 계산한 값으로 갱신하세요.
          </p>
          <SmallHashField
            hash={strategy.verification.hash}
            onChange={(hash) => onChange({ ...strategy, verification: { kind: "sha256", hash } })}
          />
        </div>
      )}
      {strategy?.kind === "github_latest_release" && (
        <div className="space-y-2 rounded border border-neutral-800 bg-neutral-950/40 p-2">
          <TextInput
            value={strategy.repo}
            placeholder="예: owner/repo"
            onChange={(repo) => onChange({ ...strategy, repo })}
          />
          <TextInput
            value={strategy.asset_name_pattern}
            placeholder="release 자산 이름 글롭 (예: *-windows-x64.zip)"
            onChange={(asset_name_pattern) => onChange({ ...strategy, asset_name_pattern })}
          />
        </div>
      )}
    </div>
  );
}

/** `ArtifactVerification`(현재 sha256 하나뿐)의 해시 직접 입력 — 압축 다운로드
 * 편집기(`RecipeEditDialog`)와 달리 "지금 이 컴퓨터에 있는 파일"을 고를 전제가
 * 없어(등록 시점엔 아직 받지도 않은 미래의 다운로드 결과) 파일 선택 계산 버튼을
 * 못 씀 — 텍스트 입력만 제공. */
function SmallHashField({ hash, onChange }: { hash: string; onChange: (h: string) => void }) {
  return <TextInput value={hash} placeholder="SHA256 해시 (64자리 hex)" onChange={onChange} />;
}

type ReadinessKind = ReadinessSignal["kind"] | "none";

const READINESS_KIND_LABELS: Record<ReadinessKind, string> = {
  none: "지정 안 함",
  child_process_window: "자식 프로세스 창 감지",
};

function ReadinessSignalFields({
  signal,
  onChange,
}: {
  signal: ReadinessSignal | null;
  onChange: (s: ReadinessSignal | null) => void;
}) {
  const kind: ReadinessKind = signal?.kind ?? "none";
  return (
    <div className="space-y-2">
      <Select
        value={kind}
        onChange={(k) =>
          onChange(k === "none" ? null : { kind: "child_process_window", cmdline_contains: "" })
        }
        options={(Object.keys(READINESS_KIND_LABELS) as ReadinessKind[]).map((k) => ({
          value: k,
          label: READINESS_KIND_LABELS[k],
        }))}
      />
      {signal?.kind === "child_process_window" && (
        <TextInput
          value={signal.cmdline_contains}
          placeholder="명령줄에 포함될 문자열"
          onChange={(cmdline_contains) => onChange({ kind: "child_process_window", cmdline_contains })}
        />
      )}
    </div>
  );
}

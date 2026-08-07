// 상대경로 목록을 폴더 트리로 보여주고, 그 안의 파일/폴더 하나를 클릭해서 골라
// 채우는 모달 — `RecipeEditDialog`(선언된 `Recipe.files` 트리)와
// `ThirdPartyAppEditDialog`(실제 폴더를 스캔한 결과 트리) 두 곳에서 공유.

import { useMemo, useState } from "react";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";
import { useDraggablePosition } from "@/lib/use-draggable-position";

export interface FileTreeFolder {
  name: string;
  path: string;
  folders: Map<string, FileTreeFolder>;
  files: { name: string; index: number }[];
}

/** 슬래시 구분 상대경로 목록을 폴더 트리로 파싱하는 순수 함수. 빈 경로(아직 안 채운
 * 항목)는 루트에 이름 없는 파일로 들어감 — 트리에서 "(이름 없음)"으로 표시. 마지막
 * 세그먼트(파일명)는 비어 있어도 그대로 파일명으로 쓴다(예: "SampleApp/"는 SampleApp
 * 폴더 밑의 이름 없는 파일) — 폴더 체인 세그먼트만 빈 문자열을 건너뛴다.
 *
 * `extraFolderPaths`(선택) — 파일이 하나도 없어도 폴더 노드를 만들어야 하는 경로들.
 * `RecipeEditDialog`가 `Recipe.folder_rules`에 등록된 경로를 넘겨서, 규칙만 걸려있고
 * 파일은 아직(또는 더 이상) 없는 폴더도 트리에서 계속 보이게 한다 — 폴더 자체는
 * 별도 스키마 필드 없이 항상 경로의 파생 구조이므로, "빈 폴더를 남긴다"는 그 경로에
 * 뭔가(파일이든 규칙이든) 하나라도 걸려있어야만 가능하다. */
export function buildFileTree(
  files: { path: string }[],
  extraFolderPaths: string[] = [],
): FileTreeFolder {
  const root: FileTreeFolder = { name: "", path: "", folders: new Map(), files: [] };

  const ensureFolder = (path: string): FileTreeFolder => {
    if (!path) return root;
    let cur = root;
    for (const seg of path.split("/")) {
      if (seg.length === 0) continue;
      let next = cur.folders.get(seg);
      if (!next) {
        next = { name: seg, path: cur.path ? `${cur.path}/${seg}` : seg, folders: new Map(), files: [] };
        cur.folders.set(seg, next);
      }
      cur = next;
    }
    return cur;
  };

  files.forEach((f, index) => {
    const segments = f.path.split("/");
    const parentPath = segments.slice(0, -1).filter((s) => s.length > 0).join("/");
    const parent = ensureFolder(parentPath);
    const name = segments[segments.length - 1];
    parent.files.push({ name, index });
  });

  for (const path of extraFolderPaths) {
    ensureFolder(path);
  }

  return root;
}

/** `basePath`가 `existingPaths`에 이미 있으면 "이름 (2)" 형태로 충돌을 피한다
 * (파일이면 확장자 보존, 폴더면 이름 전체 뒤에 붙음 — 확장자 판정은 마지막
 * 세그먼트에 `.`이 있는지로만 하므로 둘 다 같은 로직으로 처리됨). 목적지 트리의
 * 드래그 이동/붙여넣기/새 폴더 생성이 목적지 경로를 계산할 때 공용으로 쓴다. */
export function uniqueTreePath(basePath: string, existingPaths: Set<string>): string {
  if (!existingPaths.has(basePath)) return basePath;
  const lastSlash = basePath.lastIndexOf("/");
  const dir = lastSlash >= 0 ? basePath.slice(0, lastSlash + 1) : "";
  const name = lastSlash >= 0 ? basePath.slice(lastSlash + 1) : basePath;
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  const ext = dot > 0 ? name.slice(dot) : "";
  for (let i = 2; ; i++) {
    const candidate = `${dir}${stem} (${i})${ext}`;
    if (!existingPaths.has(candidate)) return candidate;
  }
}

function collectFolderPaths(folder: FileTreeFolder, out: Set<string>): void {
  for (const f of folder.folders.values()) {
    out.add(f.path);
    collectFolderPaths(f, out);
  }
}

/** 파일 경로 목록 + 트리에서 파생된 폴더 경로 전부를 하나의 집합으로 — 새 경로가
 * 기존 파일/폴더 어느 쪽과도 안 겹치는지 확인할 때(`uniqueTreePath`와 함께) 씀. */
export function collectExistingTreePaths(paths: string[], tree: FileTreeFolder): Set<string> {
  const out = new Set(paths);
  collectFolderPaths(tree, out);
  return out;
}

export type PathPickerMode = "file" | "folder";

/** 경로를 손으로 타이핑하는 대신 트리에서 직접 골라 채울 수 있게 하는 모달 —
 * 오타 없이 실제 존재하는 경로만 고를 수 있다. OS 파일 선택 다이얼로그의
 * "파일 선택"/"폴더 선택"처럼 `mode`로 무엇을 고를 수 있는지 가른다: `"file"`은
 * 트리의 파일(리프)만 클릭 가능하고 폴더는 탐색용 라벨일 뿐이고, `"folder"`는
 * 반대로 폴더 이름 자체를 클릭해서 고르며 파일은 안 보여준다(맨 위 "(루트)"로
 * 최상위 폴더도 고를 수 있음).
 *
 * 트리거 위치에 붙는 앵커 팝오버가 아니라 `Portal`로 화면 중앙에 뜨는 독립 모달 —
 * 이 컴포넌트를 여는 버튼이 항상 `RecipeEditDialog`/`ThirdPartyAppEditDialog`
 * 같은 스크롤 가능한 부모 다이얼로그 안에 있어서, 앵커 팝오버였을 땐 부모의
 * 스크롤 박스에 잘리거나 트리가 깊으면 선택 자체가 힘들었다(2026-08 실사용
 * 피드백). 독립 모달이라 부모 레이아웃/스크롤과 완전히 무관하게 화면 전체를
 * 쓸 수 있다. */
export function DestinationPathPicker({
  files,
  mode,
  onPick,
  emptyLabel,
  extraFolderPaths,
}: {
  files: { path: string }[];
  mode: PathPickerMode;
  onPick: (path: string) => void;
  /** 트리가 비어있을 때 안내 문구 — 호출부마다 트리의 출처(선언된 목록 vs 폴더
   * 스캔 결과)가 달라 문구도 다르다. */
  emptyLabel?: string;
  /** 파일이 하나도 없어도 폴더로 나타나야 하는 경로들(`buildFileTree` 참고) —
   * `RecipeEditDialog`가 `Recipe.folder_rules` 경로를 넘겨서, 규칙만 걸려있고
   * 아직 파일은 없는 폴더도 이 "찾아보기" 모달에서 고를 수 있게 한다. */
  extraFolderPaths?: string[];
}) {
  const [open, setOpen] = useState(false);
  const { style: dragStyle, onHeaderMouseDown } = useDraggablePosition(open);
  const tree = useMemo(() => buildFileTree(files, extraFolderPaths), [files, extraFolderPaths]);
  const isEmpty = tree.folders.size === 0 && tree.files.length === 0;

  return (
    <>
      <Button
        type="button"
        size="sm"
        variant="outline"
        onClick={() => setOpen(true)}
        className="shrink-0 cursor-pointer"
      >
        찾아보기
      </Button>
      {open && (
        <Portal>
          <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-4">
            <div
              className="flex max-h-[70vh] w-full max-w-md flex-col rounded-lg border border-neutral-800 bg-neutral-900 p-3 shadow-2xl"
              style={dragStyle}
              onClick={(e) => e.stopPropagation()}
            >
              <div
                className="mb-2 flex items-center justify-between gap-2"
                onMouseDown={onHeaderMouseDown}
              >
                <p className="text-xs font-medium text-neutral-300">
                  {mode === "folder" ? "폴더 선택" : "파일 선택"}
                </p>
                <button
                  type="button"
                  onClick={() => setOpen(false)}
                  onMouseDown={(e) => e.stopPropagation()}
                  className="cursor-pointer text-xs text-neutral-500 hover:text-neutral-300"
                >
                  닫기
                </button>
              </div>
              <div className="overflow-y-auto">
                {mode === "folder" && (
                  <button
                    type="button"
                    onClick={() => {
                      onPick("");
                      setOpen(false);
                    }}
                    className="block w-full cursor-pointer truncate rounded px-1 py-0.5 text-left text-xs text-neutral-300 hover:bg-neutral-800"
                  >
                    (루트)
                  </button>
                )}
                {isEmpty ? (
                  <p className="px-1 py-1 text-xs text-neutral-500">
                    {emptyLabel ?? (mode === "folder" ? "아직 하위 폴더가 없습니다." : "아직 대상이 없습니다.")}
                  </p>
                ) : (
                  <DestinationPickerFolder
                    folder={tree}
                    depth={0}
                    mode={mode}
                    onPick={(path) => {
                      onPick(path);
                      setOpen(false);
                    }}
                  />
                )}
              </div>
            </div>
          </div>
        </Portal>
      )}
    </>
  );
}

function DestinationPickerFolder({
  folder,
  depth,
  mode,
  onPick,
}: {
  folder: FileTreeFolder;
  depth: number;
  mode: PathPickerMode;
  onPick: (path: string) => void;
}) {
  const sortedFolders = Array.from(folder.folders.values()).sort((a, b) => a.name.localeCompare(b.name));
  const sortedFiles = [...folder.files].sort((a, b) => a.name.localeCompare(b.name));
  return (
    <div>
      {sortedFolders.map((f) => (
        <DestinationPickerFolderNode key={f.path} folder={f} depth={depth} mode={mode} onPick={onPick} />
      ))}
      {mode === "file" &&
        sortedFiles.map((file) => {
          const path = folder.path ? `${folder.path}/${file.name}` : file.name;
          return (
            <button
              key={file.index}
              type="button"
              onClick={() => onPick(path)}
              className="block w-full cursor-pointer truncate rounded px-1 py-0.5 text-left text-xs text-neutral-300 hover:bg-neutral-800"
              style={{ paddingLeft: depth * 12 + 14 }}
            >
              📄 {file.name || "(이름 없음)"}
            </button>
          );
        })}
    </div>
  );
}

/** 접고 펼 수 있는 폴더 한 칸. 화살표는 펼침/접힘만 담당하고, 폴더 이름 자체를
 * 클릭하는 것(폴더 모드일 때만 고를 수 있음)과는 분리돼 있다. */
function DestinationPickerFolderNode({
  folder,
  depth,
  mode,
  onPick,
}: {
  folder: FileTreeFolder;
  depth: number;
  mode: PathPickerMode;
  onPick: (path: string) => void;
}) {
  const [expanded, setExpanded] = useState(true);
  return (
    <div>
      <div className="flex items-center gap-1 rounded px-1 py-0.5 hover:bg-neutral-800/50" style={{ paddingLeft: depth * 12 }}>
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          className="w-3 shrink-0 cursor-pointer text-[10px] text-neutral-500"
        >
          {expanded ? "▾" : "▸"}
        </button>
        {mode === "folder" ? (
          <button
            type="button"
            onClick={() => onPick(folder.path)}
            className="flex-1 cursor-pointer truncate text-left text-xs text-neutral-300 hover:text-neutral-100"
          >
            📁 {folder.name}
          </button>
        ) : (
          <span className="flex-1 truncate text-xs text-neutral-400">📁 {folder.name}</span>
        )}
      </div>
      {expanded && <DestinationPickerFolder folder={folder} depth={depth + 1} mode={mode} onPick={onPick} />}
    </div>
  );
}

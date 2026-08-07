// 목적지 트리 — RecipeEditDialog "파일" 탭의 미니 파일탐색기. `Recipe.files`(경로
// 문자열 목록)에서 파생되는 가상 트리 위에서 선택/추가/삭제뿐 아니라 OS 탐색기
// 수준의 조작(다중 선택/드래그 이동/우클릭 메뉴/자르기·복사·붙여넣기/단축키)을
// 지원한다.
//
// "폴더"는 별도 엔티티가 아니라 경로 접두사로만 존재(`buildFileTree` 참고) — 이동/
// 이름변경은 그 접두사를 가진 모든 항목의 경로를 일괄 재작성하는 것과 같다. 이
// 파일은 "어디로 옮길지(`to`) 계산"까지만 하고, 실제 재작성(`moveTreePath`/
// `duplicateTreePath` — archives의 `extract_to`/`path_overrides`/`launch.entry_point`
// 까지 같이 갱신해야 함)은 `RecipeEditDialog.tsx`가 소유한다 — 그쪽만 전체 `Recipe`를
// 갖고 있기 때문. 선택 상태(`selectedKeys`)도 같은 이유로 `RecipeEditDialog.tsx`가
// 소유(controlled) — 오른쪽 편집 폼이 "몇 개가 선택됐는지"를 알아야 하고, 삭제 등
// 트리 밖에서 트리거되는 조작도 같은 선택 상태를 정확히 갱신해야 하기 때문이다.

import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Portal } from "@/components/ui/portal";
import {
  collectExistingTreePaths,
  uniqueTreePath,
  type FileTreeFolder,
} from "@/components/ui/file-tree-picker";
import type { FolderRule, FolderRuleMode, OptionalGroup, OverrideContent, RecipeFile } from "@/lib/library";

/** [`RecipeFile.override_content`]의 종류 — "없음"까지 포함해 편집 UI(드롭다운·
 * 트리 배지)에서 같이 쓰는 표시용 타입. `RecipeEditDialog.tsx`의
 * `OverrideContentFields`도 이걸 그대로 가져다 쓴다(두 파일이 서로 import하는
 * 순환을 피하려고 이쪽이 소유). */
export type OverrideKind = OverrideContent["kind"] | "none";

export const OVERRIDE_KIND_LABELS: Record<OverrideKind, string> = {
  none: "원본 유지",
  literal: "일괄 변경",
};

function basename(path: string): string {
  const i = path.lastIndexOf("/");
  return i >= 0 ? path.slice(i + 1) : path;
}

function dirname(path: string): string {
  const i = path.lastIndexOf("/");
  return i >= 0 ? path.slice(0, i) : "";
}

function joinPath(dir: string, name: string): string {
  return dir ? `${dir}/${name}` : name;
}

/** 선택 상태(`RecipeEditDialog.tsx`가 `Set<string>`으로 소유)와 트리 행(`TreeRow.key`)이
 * 공유하는 식별자 형식 — 두 파일이 서로 다른 문자열 리터럴로 이 포맷을 흉내내다
 * 어긋나는 걸 막기 위해 여기 한 곳에서만 만들고 파싱한다. 파일은 인덱스 기준(빈
 * 경로 파일이 여러 개 동시에 있을 수 있어 경로만으론 유일성이 안 보장됨), 폴더는
 * 경로 기준(폴더는 파생 구조라 인덱스 개념이 없음). */
export function fileKey(index: number): string {
  return `file:${index}`;
}
export function folderKey(path: string): string {
  return `folder:${path}`;
}
export type TreeSelectionKey = { kind: "file"; index: number } | { kind: "folder"; path: string };
export function parseTreeKey(key: string): TreeSelectionKey {
  if (key.startsWith("folder:")) return { kind: "folder", path: key.slice("folder:".length) };
  return { kind: "file", index: Number(key.slice("file:".length)) };
}

/** 트리 한 행 — 폴더 또는 파일. `flattenVisibleTreeRows`가 만드는 평평한 배열의 원소. */
interface TreeRow {
  key: string;
  depth: number;
  data: { kind: "folder"; folder: FileTreeFolder } | { kind: "file"; name: string; index: number };
}

/** 트리를 지금 펼쳐진 상태(`collapsed`) 기준으로 "실제로 보이는 행"만 평평한 배열로
 * 만든다 — 이 배열에 `useVirtualizer`를 적용해 화면에 실제 보이는 몇십 개 행만
 * DOM에 그린다. 수백~수천 개 파일이 있는 레시피(예: 대용량 모드팩)에서 트리 전체를
 * 그대로 그리면, 파일 하나만 지워도 그 많은 노드를 전부 재조정해야 해서 눈에 띄게
 * 느려짐 — 실사용 중 발견. 펼침/접힘 상태를 폴더별 컴포넌트 로컬 state 대신
 * `FileTreeView`가 경로 집합으로 들고 있는 이유도 이 평탄화 때문 — 컴포넌트 트리
 * 밖에서 "지금 뭐가 보이는가"를 알아야 배열을 만들 수 있다. Shift+범위 선택/Ctrl+A도
 * 이 평평한 순서를 기준으로 계산한다. */
function flattenVisibleTreeRows(folder: FileTreeFolder, collapsed: Set<string>, depth = 0): TreeRow[] {
  const sortedFolders = Array.from(folder.folders.values()).sort((a, b) => a.name.localeCompare(b.name));
  const sortedFiles = [...folder.files].sort((a, b) => a.name.localeCompare(b.name));
  const rows: TreeRow[] = [];
  for (const f of sortedFolders) {
    rows.push({ key: folderKey(f.path), depth, data: { kind: "folder", folder: f } });
    if (!collapsed.has(f.path)) {
      rows.push(...flattenVisibleTreeRows(f, collapsed, depth + 1));
    }
  }
  for (const file of sortedFiles) {
    rows.push({ key: fileKey(file.index), depth, data: { kind: "file", name: file.name, index: file.index } });
  }
  return rows;
}

const TREE_ROW_HEIGHT = 22;

type DragSource = { kind: "file"; path: string } | { kind: "folder"; path: string };

/** 트리 드래그 이동 — 압축 목록의 재정렬(간격에 끼워넣기, 형제 순서 변경)과 달리
 * 트리는 알파벳 정렬 고정이라 형제 순서 개념이 없다. "폴더 위에 드롭 = 그 폴더의
 * 자식이 된다"만 있으면 됨 — 목적지가 항상 폴더(또는 루트) 하나뿐이라 gap-index
 * 계산이 불필요해서 별도 훅으로 분리했다(가장자리 자동 스크롤만 압축 목록과 같은
 * interval 패턴 — 드롭 판정 로직 자체가 달라 공용화는 과함, 세 줄 수준 중복은
 * 추상화보다 낫다는 컨벤션). 다중 선택 상태에서 그중 하나를 끌면 선택된 것 전부를
 * 옮기므로 `dragSources`는 배열이다. */
function useTreeDragMove(onDrop: (sources: DragSource[], targetFolderPath: string) => void) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [dragSources, setDragSources] = useState<DragSource[]>([]);
  // 실제 이동 대상 폴더 경로(드롭 시 이걸로 계산) — 파일 행 위에 있어도 그 파일의
  // 부모 폴더가 여기 들어간다.
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  // 시각 하이라이트 대상 — 지금 커서가 정확히 올라가 있는 행 하나(`TreeRow.key`).
  // `dropTarget`(폴더 경로)로 하이라이트를 판정하면, 같은 폴더 안의 형제 파일
  // 전부가 "그 폴더가 드롭 대상"이라는 조건을 동시에 만족해 전부 파란색으로
  // 바뀌는 문제가 있었다(2026-08 실사용 버그) — 그래서 "어디로 옮길지"와 "뭘
  // 하이라이트할지"를 별개 상태로 분리했다.
  const [hoveredRowKey, setHoveredRowKey] = useState<string | null>(null);
  const [scrollDir, setScrollDir] = useState<-1 | 0 | 1>(0);

  useEffect(() => {
    if (scrollDir === 0) return;
    const id = setInterval(() => {
      containerRef.current?.scrollBy({ top: scrollDir * 14 });
    }, 16);
    return () => clearInterval(id);
  }, [scrollDir]);

  const reset = () => {
    setDragSources([]);
    setDropTarget(null);
    setHoveredRowKey(null);
    setScrollDir(0);
  };

  /** `target`이 끌고 있는 항목 중 하나 자신이거나, 끌고 있는 어떤 폴더의 하위면
   * 거부 — 폴더 여러 개를 같이 끌 때 서로가 서로의 함정이 되지 않게 전부 검사. */
  const canDropOn = (target: string): boolean => {
    if (dragSources.length === 0) return false;
    return dragSources.every(
      (src) => !(src.kind === "folder" && (target === src.path || target.startsWith(`${src.path}/`))),
    );
  };

  const handleDragStart = (sources: DragSource[]) => setDragSources(sources);

  const handleContainerDragOver = (e: React.DragEvent<HTMLDivElement>) => {
    if (dragSources.length === 0) return;
    e.preventDefault();
    const rect = containerRef.current?.getBoundingClientRect();
    if (rect) {
      const EDGE = 28;
      if (e.clientY - rect.top < EDGE) setScrollDir(-1);
      else if (rect.bottom - e.clientY < EDGE) setScrollDir(1);
      else setScrollDir(0);
    }
    // 더 구체적인 행(폴더) 위가 아니라 컨테이너 빈 공간 위에 직접 떠 있을 때만
    // "루트로 드롭"을 하이라이트 — 폴더 행의 dragOver 는 stopPropagation 안 해서
    // 이 핸들러까지 버블링되므로, target===currentTarget 로 구분한다.
    if (e.target === e.currentTarget && canDropOn("")) {
      setDropTarget("");
      setHoveredRowKey(null);
    }
  };

  /** 폴더 행 위에서든(그 폴더 자체) 파일 행 위에서든(그 파일이 속한 폴더로 귀결
   * — 호출부가 `dirname`으로 미리 계산해서 넘김) 공용으로 쓰는 드롭 대상 판정.
   * `rowKey`는 하이라이트 판정 전용(`TreeRow.key`) — 실제 이동 대상(`targetPath`)과
   * 분리된 이유는 위 `hoveredRowKey` 주석 참고. */
  const handleRowDragOver = (targetPath: string, rowKey: string, e: React.DragEvent<HTMLDivElement>) => {
    if (dragSources.length === 0 || !canDropOn(targetPath)) return;
    e.preventDefault();
    setDropTarget(targetPath);
    setHoveredRowKey(rowKey);
  };

  const handleDrop = (targetPath: string, e: React.DragEvent<HTMLDivElement>) => {
    e.preventDefault();
    e.stopPropagation();
    if (dragSources.length > 0 && canDropOn(targetPath)) {
      onDrop(dragSources, targetPath);
    }
    reset();
  };

  const handleContainerDrop = (e: React.DragEvent<HTMLDivElement>) => {
    // 폴더 행에서 stopPropagation 된 드롭은 여기까지 안 옴 — 여기 도달했다는 것
    // 자체가 "폴더 행이 아닌 곳(=루트)에 놓았다"는 뜻.
    handleDrop("", e);
  };

  return {
    containerRef,
    dragSources,
    dropTarget,
    hoveredRowKey,
    handleDragStart,
    handleContainerDragOver,
    handleRowDragOver,
    handleDrop,
    handleContainerDrop,
    handleDragEnd: reset,
  };
}

/** 우클릭 대상 — 파일/폴더 행이거나, 트리의 빈 영역(루트). */
type ContextMenuTarget =
  | { kind: "file"; path: string; index: number }
  | { kind: "folder"; path: string }
  | { kind: "root" };

interface ContextMenuState {
  x: number;
  y: number;
  target: ContextMenuTarget;
}

/** 우클릭 컨텍스트 메뉴 — `AppCard.tsx`의 `CardMenu`(Portal + `fixed` 좌표 +
 * 바깥클릭/Escape/스크롤로 닫힘)와 같은 패턴이되, 트리거가 버튼이 아니라 우클릭
 * 좌표(호출부가 이미 화면 밖으로 안 나가게 clamp 해서 넘김). `selectionCount`가
 * 2 이상이면(다중 선택) 새 파일/새 폴더/이름 바꾸기는 의미가 없어 숨긴다 — 잘라
 * 내기/복사/삭제만 선택된 것 전부에 적용된다. */
function TreeContextMenu({
  state,
  onClose,
  clipboardHasContent,
  selectionCount,
  onRename,
  onCut,
  onCopy,
  onPaste,
  onDelete,
  onNewFile,
  onNewFolder,
}: {
  state: ContextMenuState;
  onClose: () => void;
  clipboardHasContent: boolean;
  selectionCount: number;
  onRename: () => void;
  onCut: () => void;
  onCopy: () => void;
  onPaste: () => void;
  onDelete: () => void;
  onNewFile: () => void;
  onNewFolder: () => void;
}) {
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    const onScrollOrResize = () => onClose();
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
  }, [onClose]);

  const isMulti = selectionCount > 1;
  const isNode = state.target.kind !== "root";
  const isFolderish = state.target.kind === "folder" || state.target.kind === "root";

  const item = (label: string, onClick: () => void, opts?: { danger?: boolean; disabled?: boolean }) => (
    <button
      type="button"
      role="menuitem"
      disabled={opts?.disabled}
      onClick={() => {
        onClick();
        onClose();
      }}
      className={`flex w-full items-center px-3 py-1.5 text-left text-xs transition-colors ${
        opts?.disabled
          ? "text-neutral-600"
          : `cursor-pointer hover:bg-neutral-800/60 ${opts?.danger ? "text-red-300" : "text-neutral-200"}`
      }`}
    >
      {label}
    </button>
  );

  return (
    <Portal>
      <div
        ref={menuRef}
        role="menu"
        className="fixed z-[70] w-48 overflow-hidden rounded-md border border-neutral-700 bg-neutral-900 py-1 shadow-lg"
        style={{ top: state.y, left: state.x }}
      >
        {isFolderish && !isMulti && item("새 파일", onNewFile)}
        {isFolderish && !isMulti && item("새 폴더", onNewFolder)}
        {isNode && !isMulti && item("이름 바꾸기", onRename)}
        {isNode && item(isMulti ? `잘라내기 (${selectionCount}개)` : "잘라내기", onCut)}
        {isNode && item(isMulti ? `복사 (${selectionCount}개)` : "복사", onCopy)}
        {isFolderish && item("붙여넣기", onPaste, { disabled: !clipboardHasContent })}
        {isNode && item(isMulti ? `삭제 (${selectionCount}개)` : "삭제", onDelete, { danger: true })}
      </div>
    </Portal>
  );
}

/** 인라인 이름변경 입력 — F2 또는 컨텍스트 메뉴 "이름 바꾸기"로 진입(다중 선택
 * 중엔 진입 자체가 막힘). Enter/blur 커밋, Escape 취소. `onKeyDown`에서
 * stopPropagation 하는 이유는 상위 트리 전체 단축키(Delete 등)가 이 입력 중엔
 * 절대 안 끼어들게 하기 위함(문서 레벨 리스너는 `document.activeElement` 검사로
 * 한 번 더 막지만, 여기서도 확실히). */
function InlineRenameInput({
  initialValue,
  onCommit,
  onCancel,
}: {
  initialValue: string;
  onCommit: (value: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initialValue);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  const commit = () => {
    const trimmed = value.trim();
    if (trimmed && trimmed !== initialValue) onCommit(trimmed);
    else onCancel();
  };

  return (
    <input
      ref={inputRef}
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onBlur={commit}
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") commit();
        else if (e.key === "Escape") onCancel();
      }}
      className="min-w-0 flex-1 rounded border border-neutral-600 bg-neutral-900 px-1 py-0 text-xs text-neutral-100 outline-none"
    />
  );
}

export function FileTreeView({
  root,
  files,
  optionalGroups,
  folderRules,
  selectedKeys,
  selectionAnchor,
  onSelectionChange,
  onAddAt,
  onDeleteSelected,
  onMove,
  onDuplicate,
  onCreateFolder,
}: {
  root: FileTreeFolder;
  files: RecipeFile[];
  optionalGroups: OptionalGroup[];
  folderRules: FolderRule[];
  /** 선택된 행들 — 키 형식은 `fileKey`/`folderKey`. 부모(`RecipeEditDialog.tsx`)가
   * 소유하는 controlled 값(오른쪽 편집 폼이 같은 값을 보고 0개/1개/여러 개 상태를
   * 가르기 때문). */
  selectedKeys: Set<string>;
  /** Shift+클릭 범위 선택의 기준점. */
  selectionAnchor: string | null;
  onSelectionChange: (keys: Set<string>, anchor: string | null) => void;
  /** 컨텍스트 메뉴 "새 파일" 전용 — 행/루트의 인라인 +/새 파일 추가 버튼은 이제
   * 없음(우클릭 메뉴와 단축키로 대체돼 중복이라 제거, 2026-08). */
  onAddAt: (folderPath: string) => void;
  /** Delete 키 / 컨텍스트 메뉴 "삭제" — 지금 `selectedKeys` 전체(1개든 여러 개든)를
   * 지운다. 행 자신의 개별 삭제도 이제 여기로 통일(선택을 그 행 하나로 좁힌 뒤
   * 호출) — 인라인 ✕ 버튼은 제거됐다. */
  onDeleteSelected: () => void;
  /** 드래그 드롭 + 인라인 이름변경(같은 부모 안에서의 `to`) + 붙여넣기(잘라내기
   * 모드) 전부 이걸로 통일 — 다중 선택 드래그/붙여넣기도 항상 배열 하나로 한 번만
   * 호출한다(반복 호출은 각 호출이 같은 렌더의 draft 스냅샷을 읽어 앞선 호출을
   * 덮어쓰는 위험이 있음 — `RecipeEditDialog.tsx`의 `handleMoveMany` 참고). 실제
   * 경로 재작성(`moveTreePath`, archives/launch까지 갱신)은 호출자 책임. */
  onMove: (moves: { from: string; to: string }[]) => void;
  /** 붙여넣기(복사 모드) — 위와 같은 이유로 배열 하나. */
  onDuplicate: (pairs: { from: string; to: string }[]) => void;
  /** `folder_rules`에 새 빈 폴더를 만들고, 실제로 생성된 경로를 돌려준다(충돌
   * 회피로 요청한 이름과 달라질 수 있음) — 호출자가 그 경로로 바로 인라인
   * 이름변경을 시작한다. */
  onCreateFolder: (parentPath: string) => string;
}) {
  // 접힌 폴더의 경로 집합 — 없으면(기본) 펼쳐진 것으로 취급. `root`(트리 자체)는
  // 파일이 바뀔 때마다 새로 만들어지지만 경로 문자열은 안정적이라 이 state는
  // 그 사이에도 그대로 유지된다(수정 중 펼침 상태가 안 흐트러짐).
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const toggleCollapsed = (path: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const [renamingPath, setRenamingPath] = useState<string | null>(null);
  const [clipboard, setClipboard] = useState<{ mode: "cut" | "copy"; keys: string[] } | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);

  const rows = useMemo(() => flattenVisibleTreeRows(root, collapsed), [root, collapsed]);
  const isEmpty = rows.length === 0;

  const existingPaths = useMemo(
    () => collectExistingTreePaths(files.map((f) => f.path), root),
    [files, root],
  );

  /** 선택 키 하나를 실제 경로로 — 파일은 지금 `files` 배열에서 인덱스로 조회(그
   * 사이 이름이 바뀌었을 수 있으니 항상 최신 경로), 폴더는 키 자체가 경로. 이미
   * 지워진 파일의 키처럼 더 이상 유효하지 않으면 `null`. */
  const resolveKeyPath = (key: string): string | null => {
    const parsed = parseTreeKey(key);
    return parsed.kind === "folder" ? parsed.path : (files[parsed.index]?.path ?? null);
  };

  /** 클릭/Ctrl+클릭/Shift+클릭의 실제 판정 — 표준 탐색기 관례: 그냥 클릭은 단일
   * 선택(그 자리가 새 기준점), Ctrl/Cmd+클릭은 그 항목만 토글(기준점도 갱신),
   * Shift+클릭은 기준점부터 지금까지 화면에 보이는 순서로 범위 전체를 선택(기존
   * 선택 교체, 기준점은 유지 — 다시 Shift+클릭하면 범위가 다시 계산되게). 라벨
   * 텍스트를 직접 클릭할 때(`handleRowClick`)와 라벨 밖 빈 공간을 움직임 없이
   * 클릭할 때(`handleRowBackgroundMouseDown`의 mouseup) 둘 다 이 판정 하나를
   * 공유 — "행 안 어디를 클릭해도 그 행이 선택된다"는 탐색기 관례를 두 경로가
   * 어긋나지 않게 하기 위함(2026-08, 실사용 피드백으로 후자 추가). */
  const applySelectionClick = (key: string, mods: { shift: boolean; ctrlOrMeta: boolean }) => {
    if (mods.shift && selectionAnchor) {
      const anchorIndex = rows.findIndex((r) => r.key === selectionAnchor);
      const clickIndex = rows.findIndex((r) => r.key === key);
      if (anchorIndex !== -1 && clickIndex !== -1) {
        const [start, end] = anchorIndex < clickIndex ? [anchorIndex, clickIndex] : [clickIndex, anchorIndex];
        onSelectionChange(new Set(rows.slice(start, end + 1).map((r) => r.key)), selectionAnchor);
        return;
      }
    }
    if (mods.ctrlOrMeta) {
      const next = new Set(selectedKeys);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      onSelectionChange(next, key);
      return;
    }
    onSelectionChange(new Set([key]), key);
  };

  const handleRowClick = (key: string, e: React.MouseEvent) => {
    applySelectionClick(key, { shift: e.shiftKey, ctrlOrMeta: e.ctrlKey || e.metaKey });
  };

  /** `sourcePath`를 뺀 나머지와만 충돌 검사 — 이름변경/이동 시 "자기 자신과 매번
   * 충돌"로 판정되는 걸 막는다. 복사(붙여넣기 복사 모드)는 원본이 그대로 남으므로
   * 이 함수를 안 쓰고 `handlePaste`에서 따로 처리(자기 자신과도 충돌해야 "복사본
   * (2)"가 만들어짐). */
  const resolveMoveTarget = (sourcePath: string, desiredPath: string): string => {
    const candidates = new Set(existingPaths);
    candidates.delete(sourcePath);
    return uniqueTreePath(desiredPath, candidates);
  };

  const drag = useTreeDragMove((sources, targetFolderPath) => {
    // 여러 개를 한 번에 드롭할 때 서로의 새 이름끼리도 충돌하면 안 되므로, 하나
    // 처리할 때마다 "이미 씀" 집합에 그 결과를 더해가며 다음 항목을 계산한다.
    const taken = new Set(existingPaths);
    for (const src of sources) taken.delete(src.path);
    const moves: { from: string; to: string }[] = [];
    for (const src of sources) {
      const desired = joinPath(targetFolderPath, basename(src.path));
      const to = uniqueTreePath(desired, taken);
      taken.add(to);
      if (to !== src.path) moves.push({ from: src.path, to });
    }
    if (moves.length > 0) onMove(moves);
  });

  const handleRowDragStart = (key: string, source: DragSource) => {
    if (selectedKeys.has(key) && selectedKeys.size > 1) {
      const sources: DragSource[] = [];
      for (const k of selectedKeys) {
        const parsed = parseTreeKey(k);
        if (parsed.kind === "folder") sources.push({ kind: "folder", path: parsed.path });
        else {
          const path = files[parsed.index]?.path;
          if (path !== undefined) sources.push({ kind: "file", path });
        }
      }
      drag.handleDragStart(sources);
    } else {
      // 선택 안 된 항목을 끌면 탐색기 관례대로 그 항목 하나로 선택이 좁혀진다.
      onSelectionChange(new Set([key]), key);
      drag.handleDragStart([source]);
    }
  };

  const openContextMenu = (e: React.MouseEvent, target: ContextMenuTarget) => {
    e.preventDefault();
    e.stopPropagation();
    if (target.kind === "root") {
      onSelectionChange(new Set(), null);
    } else {
      const key = target.kind === "folder" ? folderKey(target.path) : fileKey(target.index);
      if (!selectedKeys.has(key)) onSelectionChange(new Set([key]), key);
    }
    const MENU_W = 200;
    const MENU_H = 240;
    setContextMenu({
      x: Math.min(e.clientX, window.innerWidth - MENU_W),
      y: Math.min(e.clientY, window.innerHeight - MENU_H),
      target,
    });
  };

  /** 붙여넣기 — 잘라내기(이동) 모드는 원본 자리를 비워도 되므로 그 경로를 충돌
   * 후보에서 뺀다(같은 폴더에 다시 붙여넣으면 제자리 no-op). 복사 모드는 원본이
   * 그대로 남으므로 자기 자신과도 충돌시켜야 "이름 (2)"가 정상적으로 붙는다 —
   * 뺐다면 같은 폴더에 복사할 때 원본을 덮어쓰는 꼴이 된다. */
  const handlePaste = (targetFolderPath: string) => {
    if (!clipboard) return;
    const sourcePaths = clipboard.keys
      .map(resolveKeyPath)
      .filter((p): p is string => p !== null);
    if (sourcePaths.length === 0) {
      setClipboard(null);
      return;
    }
    const taken = new Set(existingPaths);
    if (clipboard.mode === "cut") {
      for (const p of sourcePaths) taken.delete(p);
    }
    const pairs: { from: string; to: string }[] = [];
    for (const p of sourcePaths) {
      const desired = joinPath(targetFolderPath, basename(p));
      const to = uniqueTreePath(desired, taken);
      taken.add(to);
      pairs.push({ from: p, to });
    }
    if (clipboard.mode === "copy") {
      onDuplicate(pairs);
    } else {
      const realMoves = pairs.filter(({ from, to }) => from !== to);
      if (realMoves.length > 0) onMove(realMoves);
      setClipboard(null);
    }
  };

  const handleRenameCommit = (sourcePath: string, newName: string) => {
    const to = resolveMoveTarget(sourcePath, joinPath(dirname(sourcePath), newName));
    if (to !== sourcePath) onMove([{ from: sourcePath, to }]);
    setRenamingPath(null);
  };

  // 단축키(Delete/F2/Ctrl+C/X/V/A) — 이 컴포넌트가 마운트돼 있는 동안(="파일" 탭이
  // 열려있는 동안)만 활성. input/textarea 에 포커스가 있으면(인라인 이름변경 중,
  // 또는 오른쪽 편집 폼의 텍스트 필드) 절대 가로채지 않는다 — 정상적인 텍스트
  // 편집을 방해하면 안 되므로.
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const active = document.activeElement;
      if (active && (active.tagName === "INPUT" || active.tagName === "TEXTAREA")) return;

      if ((e.ctrlKey || e.metaKey) && e.key === "a") {
        e.preventDefault();
        onSelectionChange(new Set(rows.map((r) => r.key)), rows.length > 0 ? rows[rows.length - 1].key : null);
        return;
      }
      if (e.key === "Delete" && selectedKeys.size > 0) {
        e.preventDefault();
        onDeleteSelected();
        return;
      }
      if (e.key === "F2" && selectedKeys.size === 1) {
        e.preventDefault();
        const path = resolveKeyPath([...selectedKeys][0]);
        if (path !== null) setRenamingPath(path);
        return;
      }
      if ((e.ctrlKey || e.metaKey) && selectedKeys.size > 0 && (e.key === "c" || e.key === "x")) {
        e.preventDefault();
        setClipboard({ mode: e.key === "x" ? "cut" : "copy", keys: [...selectedKeys] });
        return;
      }
      if ((e.ctrlKey || e.metaKey) && e.key === "v" && clipboard) {
        e.preventDefault();
        // 붙여넣기 대상 폴더 — 선택된 게 폴더 하나뿐이면 그 폴더 안, 아니면 루트.
        const onlyKey = selectedKeys.size === 1 ? [...selectedKeys][0] : null;
        const onlyParsed = onlyKey ? parseTreeKey(onlyKey) : null;
        const targetFolder = onlyParsed?.kind === "folder" ? onlyParsed.path : "";
        handlePaste(targetFolder);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [selectedKeys, files, clipboard, rows, onSelectionChange, onDeleteSelected, handlePaste]);

  const scrollRef = drag.containerRef;
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => TREE_ROW_HEIGHT,
    overscan: 12,
  });

  // 드래그 박스(마퀴) 선택 — 행의 라벨(아이콘+이름) 밖, 빈 공간에서 누른 채 끌면
  // 그 사각형과 겹치는 행 전부가 선택된다(파일명 텍스트 위에서 누르면 그건 기존
  // 클릭/드래그-이동 제스처가 우선). 행 높이가 고정(`TREE_ROW_HEIGHT`)이라
  // 가상화된 화면 밖 행도 DOM 없이 `index * TREE_ROW_HEIGHT` 산수만으로 겹침
  // 판정이 된다 — `virtualizer`의 실측 없이도 정확.
  const [marquee, setMarquee] = useState<{ startY: number; curY: number } | null>(null);
  const marqueeRef = useRef<{
    startY: number;
    rowKey: string | null;
    shift: boolean;
    ctrlOrMeta: boolean;
    base: Set<string>;
    moved: boolean;
  } | null>(null);

  const getContentY = (clientY: number): number => {
    const rect = scrollRef.current?.getBoundingClientRect();
    if (!rect) return 0;
    return clientY - rect.top + (scrollRef.current?.scrollTop ?? 0);
  };

  /** 라벨(아이콘+이름) 밖 빈 공간의 mousedown — `rowKey`는 어느 행 위에서
   * 눌렀는지(행 자신이 호출 시 그 행의 키를 넘김, 컨테이너의 진짜 빈 배경에서
   * 호출되면 `null`). 움직이지 않고 그냥 떼면(마퀴로 안 번지면) 탐색기 관례대로
   * "행 안 어디를 눌러도 그 행이 선택된다"를 만족시키려고 `applySelectionClick`을
   * 그대로 재사용 — 텍스트를 직접 클릭했을 때(`handleRowClick`)와 똑같은 결과가
   * 나오게(2026-08, 실사용 피드백). `rowKey`가 없으면(컨테이너 배경) 기존대로
   * 선택 해제. */
  const handleRowBackgroundMouseDown = (e: React.MouseEvent, rowKey: string | null) => {
    if (e.button !== 0) return; // 왼쪽 버튼만
    const startY = getContentY(e.clientY);
    const shift = e.shiftKey;
    const ctrlOrMeta = e.ctrlKey || e.metaKey;
    const additive = shift || ctrlOrMeta;
    marqueeRef.current = {
      startY,
      rowKey,
      shift,
      ctrlOrMeta,
      base: additive ? new Set(selectedKeys) : new Set(),
      moved: false,
    };

    const onMove = (ev: MouseEvent) => {
      const state = marqueeRef.current;
      if (!state) return;
      const curY = getContentY(ev.clientY);
      if (!state.moved && Math.abs(curY - state.startY) < 4) return; // 임계값 전엔 아직 클릭일 수도
      state.moved = true;
      setMarquee({ startY: state.startY, curY });

      const rect = scrollRef.current?.getBoundingClientRect();
      if (rect) {
        const EDGE = 28;
        if (ev.clientY - rect.top < EDGE) scrollRef.current?.scrollBy({ top: -14 });
        else if (rect.bottom - ev.clientY < EDGE) scrollRef.current?.scrollBy({ top: 14 });
      }

      const [lo, hi] = state.startY < curY ? [state.startY, curY] : [curY, state.startY];
      const overlapping = rows.filter((_, i) => {
        const top = i * TREE_ROW_HEIGHT;
        return top + TREE_ROW_HEIGHT > lo && top < hi;
      });
      const next = new Set(state.base);
      for (const r of overlapping) next.add(r.key);
      // 마퀴로 갱신된 선택은 기준점(Shift+클릭용)을 안 남긴다 — 다음 Shift+클릭은
      // 마지막으로 겹친 행이 아니라 그때 새로 클릭한 지점부터 시작하는 게 자연스럽다.
      onSelectionChange(next, null);
    };
    const onUp = () => {
      const state = marqueeRef.current;
      // 움직이지 않고 그냥 뗐으면(순수 클릭, 마퀴로 안 번짐) — 어느 행 위였는지에
      // 따라 갈린다: 행 위였으면 그 행을 선택(텍스트를 직접 클릭한 것과 동일),
      // 컨테이너의 진짜 빈 배경이었으면 기존대로 선택 해제.
      if (state && !state.moved) {
        if (state.rowKey !== null) {
          applySelectionClick(state.rowKey, { shift: state.shift, ctrlOrMeta: state.ctrlOrMeta });
        } else if (!state.ctrlOrMeta) {
          onSelectionChange(new Set(), null);
        }
      }
      marqueeRef.current = null;
      setMarquee(null);
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  };

  return (
    <div
      className="flex min-h-0 flex-1 flex-col rounded border border-neutral-800 bg-neutral-950/40 p-2"
      onContextMenu={(e) => openContextMenu(e, { kind: "root" })}
    >
      {isEmpty && <p className="px-1 py-2 text-xs text-neutral-500">아직 파일이 없습니다.</p>}
      {!isEmpty && (
        <div
          ref={scrollRef}
          className={`min-h-0 flex-1 overflow-y-auto rounded ${
            drag.dropTarget === "" ? "ring-1 ring-inset ring-blue-500/50" : ""
          }`}
          onDragOver={drag.handleContainerDragOver}
          onDrop={drag.handleContainerDrop}
          onMouseDown={(e) => handleRowBackgroundMouseDown(e, null)}
        >
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
            {marquee && (
              <div
                className="pointer-events-none absolute inset-x-0 z-10 border border-blue-400 bg-blue-500/20"
                style={{
                  top: Math.min(marquee.startY, marquee.curY),
                  height: Math.abs(marquee.curY - marquee.startY),
                }}
              />
            )}
            {virtualizer.getVirtualItems().map((virtualRow) => {
              const row = rows[virtualRow.index];
              const data = row.data;
              return (
                <div
                  key={row.key}
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    width: "100%",
                    height: TREE_ROW_HEIGHT,
                    transform: `translateY(${virtualRow.start}px)`,
                  }}
                >
                  {data.kind === "folder" ? (
                    <FolderRow
                      folder={data.folder}
                      depth={row.depth}
                      expanded={!collapsed.has(data.folder.path)}
                      onToggleExpanded={() => toggleCollapsed(data.folder.path)}
                      rule={folderRules.find((r) => r.path === data.folder.path)}
                      selected={selectedKeys.has(row.key)}
                      onSelectClick={(e) => handleRowClick(row.key, e)}
                      onRowMouseDown={(e) => {
                        // 컨테이너 자체의 같은 핸들러까지 버블링되면 마퀴가 두 번
                        // 시작되므로(리스너 중복 등록) 여기서 끊는다 — 컨테이너
                        // 쪽은 "어느 행에도 속하지 않는 빈 공간"에서만 걸리게.
                        e.stopPropagation();
                        handleRowBackgroundMouseDown(e, row.key);
                      }}
                      renaming={renamingPath === data.folder.path}
                      onCommitRename={(name) => handleRenameCommit(data.folder.path, name)}
                      onCancelRename={() => setRenamingPath(null)}
                      onContextMenu={(e) => openContextMenu(e, { kind: "folder", path: data.folder.path })}
                      isCutSource={
                        clipboard?.mode === "cut" &&
                        clipboard.keys.includes(folderKey(data.folder.path))
                      }
                      isDropTarget={drag.hoveredRowKey === row.key}
                      onDragStart={() => handleRowDragStart(row.key, { kind: "folder", path: data.folder.path })}
                      onDragOver={(e) => drag.handleRowDragOver(data.folder.path, row.key, e)}
                      onDrop={(e) => drag.handleDrop(data.folder.path, e)}
                      onDragEnd={drag.handleDragEnd}
                    />
                  ) : (
                    <FileLeaf
                      name={data.name}
                      file={files[data.index]}
                      optionalGroups={optionalGroups}
                      depth={row.depth}
                      selected={selectedKeys.has(row.key)}
                      onSelectClick={(e) => handleRowClick(row.key, e)}
                      onRowMouseDown={(e) => {
                        // 컨테이너 자체의 같은 핸들러까지 버블링되면 마퀴가 두 번
                        // 시작되므로(리스너 중복 등록) 여기서 끊는다 — 컨테이너
                        // 쪽은 "어느 행에도 속하지 않는 빈 공간"에서만 걸리게.
                        e.stopPropagation();
                        handleRowBackgroundMouseDown(e, row.key);
                      }}
                      renaming={renamingPath === files[data.index]?.path}
                      onCommitRename={(name) => handleRenameCommit(files[data.index].path, name)}
                      onCancelRename={() => setRenamingPath(null)}
                      onContextMenu={(e) =>
                        openContextMenu(e, { kind: "file", path: files[data.index].path, index: data.index })
                      }
                      isCutSource={clipboard?.mode === "cut" && clipboard.keys.includes(row.key)}
                      isDropTarget={drag.hoveredRowKey === row.key}
                      onDragStart={() =>
                        handleRowDragStart(row.key, { kind: "file", path: files[data.index].path })
                      }
                      onDragOver={(e) => drag.handleRowDragOver(dirname(files[data.index].path), row.key, e)}
                      onDrop={(e) => drag.handleDrop(dirname(files[data.index].path), e)}
                      onDragEnd={drag.handleDragEnd}
                    />
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {contextMenu && (
        <TreeContextMenu
          state={contextMenu}
          onClose={() => setContextMenu(null)}
          clipboardHasContent={clipboard !== null}
          selectionCount={selectedKeys.size}
          onRename={() => {
            if (contextMenu.target.kind !== "root") setRenamingPath(contextMenu.target.path);
          }}
          onCut={() => setClipboard({ mode: "cut", keys: [...selectedKeys] })}
          onCopy={() => setClipboard({ mode: "copy", keys: [...selectedKeys] })}
          onPaste={() => {
            const folderPath = contextMenu.target.kind === "folder" ? contextMenu.target.path : "";
            handlePaste(folderPath);
          }}
          onDelete={onDeleteSelected}
          onNewFile={() => {
            const folderPath = contextMenu.target.kind === "folder" ? contextMenu.target.path : "";
            onAddAt(folderPath);
          }}
          onNewFolder={() => {
            const folderPath = contextMenu.target.kind === "folder" ? contextMenu.target.path : "";
            setRenamingPath(onCreateFolder(folderPath));
          }}
        />
      )}
    </div>
  );
}

function FolderRow({
  folder,
  depth,
  expanded,
  onToggleExpanded,
  rule,
  selected,
  onSelectClick,
  onRowMouseDown,
  renaming,
  onCommitRename,
  onCancelRename,
  onContextMenu,
  isCutSource,
  isDropTarget,
  onDragStart,
  onDragOver,
  onDrop,
  onDragEnd,
}: {
  folder: FileTreeFolder;
  depth: number;
  expanded: boolean;
  onToggleExpanded: () => void;
  rule: FolderRule | undefined;
  selected: boolean;
  onSelectClick: (e: React.MouseEvent) => void;
  /** 라벨(아이콘+이름) 밖 빈 공간에서 누르면 드래그 박스(마퀴) 선택이 시작된다 —
   * 라벨은 `max-w-[70%]`로 폭을 제한해서 짧은 이름 뒤든 긴 이름이 잘린 뒤든 항상
   * 빈 공간이 남게 한다. */
  onRowMouseDown: (e: React.MouseEvent) => void;
  renaming: boolean;
  onCommitRename: (newName: string) => void;
  onCancelRename: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
  isCutSource: boolean;
  isDropTarget: boolean;
  onDragStart: () => void;
  onDragOver: (e: React.DragEvent<HTMLDivElement>) => void;
  onDrop: (e: React.DragEvent<HTMLDivElement>) => void;
  onDragEnd: () => void;
}) {
  return (
    <div
      onDragOver={onDragOver}
      onDrop={onDrop}
      onContextMenu={onContextMenu}
      onMouseDown={onRowMouseDown}
      className={`group flex h-full items-center gap-1 rounded px-1 ${
        selected
          ? "bg-neutral-800"
          : isDropTarget
            ? "bg-blue-900/30 ring-1 ring-blue-500/50"
            : "hover:bg-neutral-800/50"
      } ${isCutSource ? "opacity-40" : ""}`}
      style={{ paddingLeft: depth * 14 }}
    >
      <button
        type="button"
        onClick={onToggleExpanded}
        onMouseDown={(e) => e.stopPropagation()}
        className="w-4 shrink-0 cursor-pointer text-neutral-500"
      >
        {expanded ? "▾" : "▸"}
      </button>
      {renaming ? (
        <InlineRenameInput initialValue={folder.name} onCommit={onCommitRename} onCancel={onCancelRename} />
      ) : (
        <button
          type="button"
          draggable
          onDragStart={onDragStart}
          onDragEnd={onDragEnd}
          onClick={onSelectClick}
          onMouseDown={(e) => e.stopPropagation()}
          className={`max-w-[70%] shrink cursor-pointer truncate text-left text-xs ${
            selected ? "text-neutral-100" : "text-neutral-300"
          }`}
        >
          📁 {folder.name}
        </button>
      )}
      {!renaming && rule && (
        <span className="shrink-0 whitespace-nowrap rounded bg-neutral-100 px-1 text-[10px] font-medium text-neutral-900 group-hover:hidden">
          {folderRuleBadgeLabel(rule.mode)}
        </span>
      )}
    </div>
  );
}

export function folderRuleBadgeLabel(mode: FolderRuleMode): string {
  return mode.kind === "passthrough" ? "전체 허용" : "필터링";
}

function FileLeaf({
  name,
  file,
  optionalGroups,
  depth,
  selected,
  onSelectClick,
  onRowMouseDown,
  renaming,
  onCommitRename,
  onCancelRename,
  onContextMenu,
  isCutSource,
  isDropTarget,
  onDragStart,
  onDragOver,
  onDrop,
  onDragEnd,
}: {
  name: string;
  file: RecipeFile;
  optionalGroups: OptionalGroup[];
  depth: number;
  selected: boolean;
  onSelectClick: (e: React.MouseEvent) => void;
  /** 라벨(아이콘+이름) 밖 빈 공간에서 누르면 드래그 박스(마퀴) 선택이 시작된다 —
   * `FolderRow`와 같은 이유로 라벨 폭을 제한해서 항상 빈 공간이 남게 한다. */
  onRowMouseDown: (e: React.MouseEvent) => void;
  renaming: boolean;
  onCommitRename: (newName: string) => void;
  onCancelRename: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
  isCutSource: boolean;
  /** 이 파일이 속한 폴더가 지금 드롭 대상으로 잡혀있는지 — 드롭은 실제로는 이
   * 파일의 부모 폴더로 귀결된다(`dirname`, 파일 자체는 무언가를 담을 수 없음).
   * 파일들 사이(형제 목록 안)에 놓아도 "그 폴더 안"으로 정확히 인식되게 하려고
   * 폴더 행과 똑같이 드롭 대상 역할을 한다 — 안 그러면 파일 행 위 드롭이 아무데도
   * 안 걸려 컨테이너까지 버블링돼 루트로 새는 문제가 있었다(2026-08 실사용 버그). */
  isDropTarget: boolean;
  onDragStart: () => void;
  onDragOver: (e: React.DragEvent<HTMLDivElement>) => void;
  onDrop: (e: React.DragEvent<HTMLDivElement>) => void;
  onDragEnd: () => void;
}) {
  const groupLabel = file.optional_group
    ? (optionalGroups.find((g) => g.id === file.optional_group)?.label ?? file.optional_group)
    : null;
  const contentLabel = file.override_content ? OVERRIDE_KIND_LABELS[file.override_content.kind] : null;
  return (
    <div
      onDragOver={onDragOver}
      onDrop={onDrop}
      onContextMenu={onContextMenu}
      onMouseDown={onRowMouseDown}
      className={`group flex h-full items-center gap-1 rounded px-1 text-xs ${
        selected
          ? "bg-neutral-800 text-neutral-100"
          : isDropTarget
            ? "bg-blue-900/30 ring-1 ring-blue-500/50"
            : "text-neutral-400 hover:bg-neutral-800/50 hover:text-neutral-200"
      } ${isCutSource ? "opacity-40" : ""}`}
      style={{ paddingLeft: depth * 14 + 20 }}
    >
      {renaming ? (
        <InlineRenameInput initialValue={name} onCommit={onCommitRename} onCancel={onCancelRename} />
      ) : (
        <button
          type="button"
          draggable
          onDragStart={onDragStart}
          onDragEnd={onDragEnd}
          onClick={onSelectClick}
          onMouseDown={(e) => e.stopPropagation()}
          className="max-w-[70%] shrink cursor-pointer truncate text-left"
        >
          📄 {name || "(이름 없음)"}
        </button>
      )}
      {!renaming && groupLabel && (
        <span className="shrink-0 whitespace-nowrap rounded bg-neutral-100 px-1 text-[10px] font-medium text-neutral-900 group-hover:hidden">
          {groupLabel}
        </span>
      )}
      {!renaming && contentLabel && (
        <span className="shrink-0 whitespace-nowrap rounded bg-neutral-100 px-1 text-[10px] font-medium text-neutral-900 group-hover:hidden">
          {contentLabel}
        </span>
      )}
    </div>
  );
}

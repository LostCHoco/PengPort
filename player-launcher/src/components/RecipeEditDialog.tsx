// 레시피(앱) 편집 다이얼로그 — v8 스키마 기준 구조화된 폼. AppCard 의 "앱 편집" 메뉴
// (기존 수정) 또는 Library.tsx 의 "새 앱 추가" 버튼(신규 등록) 양쪽에서 연다 —
// `recipe: null` 이면 신규.
//
// 폼 구조: ① 이름 ② 압축 다운로드(`archives`, 자유 조합) ③ 파일 화이트리스트+오버라이드
// (`files`, "폴더 불러오기"로 자동 채우기 가능) ④ 실행 방법 ⑤ 부가 정보(아이콘).
// `archives`+`files` 합쳐서 최소 1개 강제. 대상 루트(App 전용 폴더냐 third-party 앱
// 데이터 영역이냐)는 항목마다 고르지 않는다 — 실행 방식(④) 하나가 결정한다.
//
// `id` 는 편집 불가 — third-party app 인스턴스 폴더명 / 앱 루트 폴더명으로 그대로
// 쓰이므로, 바꾸면 이미 설치된 폴더와 레시피가 어긋난다. 신규 등록은 `name` 에서 저장
// 시점에 1회 slugify — 사용자에게 id 입력을 아예 안 시킨다.

import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";
import { Field, inputClass, RemoveButton, Select, TextInput } from "@/components/ui/form-fields";
import { buildFileTree, DestinationPathPicker, type FileTreeFolder } from "@/components/ui/file-tree-picker";
import {
  computeFileSha256,
  listThirdPartyAppIds,
  readConfigFileAsPatch,
  scanFolderRelativePaths,
} from "@/lib/library";
import type {
  ArchiveExtraction,
  ArtifactVerification,
  ConfigFileFormat,
  ConfigPatch,
  FileContent,
  FolderRule,
  FolderRuleMode,
  LaunchAction,
  OptionalGroup,
  OverrideContent,
  PathOverride,
  Recipe,
  RecipeFile,
} from "@/lib/library";

interface Props {
  /** `null` 이면 신규 등록 — id 는 저장 시점에 `name` 에서 slugify. */
  recipe: Recipe | null;
  /** 신규 등록 시 id 충돌(같은 이름을 slugify 한 결과가 이미 있음) 회피용. */
  existingIds: string[];
  onSave: (recipe: Recipe) => Promise<void>;
  onCancel: () => void;
}

type LaunchActionKind = LaunchAction["kind"];
type OverrideKind = OverrideContent["kind"] | "none";

// 폼이 길어져(archives/files 는 항목이 수십~수백 개까지 늘어날 수 있음) 한 화면
// 세로 스크롤 대신 사이드바 탭으로 나눈다 — 탭 사이 이동은 순수 로컬 UI 상태일 뿐
// draft 자체(Recipe)는 안 바뀐다.
// 설치 옵션(Recipe.optional_groups) 관리는 별도 탭 없이 "압축 다운로드" 탭 안에
// 흡수돼 있다 — 그룹 생성부터 편집까지 대부분 압축 단위로 이뤄지는 게 실사용
// 패턴이라(개별 파일 태깅은 드문 예외), 별도 탭이 오히려 "그룹을 먼저 선언하고
// 나중에 배정" 이라는 불필요한 단계를 강제했다(2026-08, 사용자 확인).
type RecipeEditTab = "basic" | "archives" | "files" | "launch";

const TAB_ORDER: RecipeEditTab[] = ["basic", "archives", "files", "launch"];

const TAB_LABELS: Record<RecipeEditTab, string> = {
  basic: "기본 정보",
  archives: "압축 다운로드",
  files: "파일",
  launch: "실행 방식",
};

const LAUNCH_KIND_LABELS: Record<LaunchActionKind, string> = {
  spawn_process: "로컬 실행 파일 실행",
  third_party_app_launch: "서드파티 앱으로 실행",
};

const OVERRIDE_KIND_LABELS: Record<OverrideKind, string> = {
  none: "원본 유지",
  literal: "일괄 변경",
  config_patch: "일부 변경",
};

function defaultNewRecipe(): Recipe {
  return {
    id: "", // 저장 시점에 slugify(name) 으로 채워짐 — 그전까진 미완성 상태.
    name: "",
    recipe_info: {},
    archives: [defaultArchive([])],
    files: [],
    optional_groups: [],
    folder_rules: [],
    launch: defaultLaunchAction("spawn_process"),
  };
}

/** `name` → fs-safe id. `pengport_shared::ids::validate_service_id` 가 허용하는
 * `[A-Za-z0-9_-]{1,64}` 로 정규화(소문자로 통일) — 규칙이 바뀌면 이 함수도 맞춰야 함. */
function slugify(name: string): string {
  const base = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/-{2,}/g, "-")
    .replace(/^-+|-+$/g, "");
  return (base || "app").slice(0, 64);
}

/** slugify 결과가 이미 라이브러리에 있으면 `-2`, `-3`... 붙여서 회피. */
function uniqueId(base: string, existingIds: string[]): string {
  if (!existingIds.includes(base)) return base;
  for (let i = 2; ; i++) {
    const candidate = `${base}-${i}`.slice(0, 64);
    if (!existingIds.includes(candidate)) return candidate;
  }
}

/** 배열 순서 자체를 드래그로 바꿀 때 쓰는 순수 함수 — `from` 위치 항목을 뽑아
 * `to` 위치에 다시 끼워넣는다. */
function moveItem<T>(items: T[], from: number, to: number): T[] {
  const copy = [...items];
  const [moved] = copy.splice(from, 1);
  copy.splice(to, 0, moved);
  return copy;
}

/** `moveItem`으로 배열이 바뀐 뒤, 그 배열을 가리키던 선택 인덱스가 같은 항목을
 * 계속 가리키도록 보정. 이동한 항목 자신이면 새 위치로, 그 사이에서 밀린
 * 항목이면 한 칸 보정, 무관하면 그대로. */
function adjustSelectedIndexAfterMove(selected: number, from: number, to: number): number {
  if (selected === from) return to;
  if (from < selected && selected <= to) return selected - 1;
  if (to <= selected && selected < from) return selected + 1;
  return selected;
}

/** 카드 목록 드래그 재정렬 공용 로직 — "카드 위에 놓기"가 아니라 "카드 사이(간격)에
 * 놓기"로 판정한다: 커서가 카드의 위/아래 절반 중 어디 있는지로 "이 간격 인덱스에
 * 끼워넣는다"를 계산(예: 1번 카드를 2·3번 사이로 끌면 1번이 그 사이로 들어가면서
 * 2번과 자리를 바꾸는 결과). 스크롤 컨테이너 위/아래 가장자리 근처로 끌고 가면
 * 자동 스크롤도 겸해서 — 지금 화면에 안 보이는 카드와도 순서를 바꿀 수 있다. */
function useDragReorder(onReorder: (from: number, to: number) => void) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [gapIndex, setGapIndex] = useState<number | null>(null);
  const [scrollDir, setScrollDir] = useState<-1 | 0 | 1>(0);

  useEffect(() => {
    if (scrollDir === 0) return;
    const id = setInterval(() => {
      containerRef.current?.scrollBy({ top: scrollDir * 14 });
    }, 16);
    return () => clearInterval(id);
  }, [scrollDir]);

  const reset = () => {
    setDragIndex(null);
    setGapIndex(null);
    setScrollDir(0);
  };

  const handleDragStart = (i: number) => setDragIndex(i);

  const handleContainerDragOver = (e: React.DragEvent<HTMLDivElement>) => {
    if (dragIndex === null) return;
    e.preventDefault();
    const rect = containerRef.current?.getBoundingClientRect();
    if (!rect) return;
    const EDGE = 28;
    if (e.clientY - rect.top < EDGE) setScrollDir(-1);
    else if (rect.bottom - e.clientY < EDGE) setScrollDir(1);
    else setScrollDir(0);
  };

  const handleCardDragOver = (i: number, e: React.DragEvent<HTMLDivElement>) => {
    if (dragIndex === null) return;
    e.preventDefault();
    // stopPropagation 안 함 — 컨테이너의 onDragOver(가장자리 자동 스크롤 감지)도
    // 버블링으로 같이 받아야 카드 위에서도 자동 스크롤이 계속 동작한다.
    const rect = e.currentTarget.getBoundingClientRect();
    const topHalf = e.clientY - rect.top < rect.height / 2;
    setGapIndex(topHalf ? i : i + 1);
  };

  const handleDrop = () => {
    if (dragIndex !== null && gapIndex !== null) {
      const to = gapIndex > dragIndex ? gapIndex - 1 : gapIndex;
      if (to !== dragIndex) onReorder(dragIndex, to);
    }
    reset();
  };

  return {
    containerRef,
    dragIndex,
    gapIndex,
    handleDragStart,
    handleContainerDragOver,
    handleCardDragOver,
    handleDrop,
    handleDragEnd: reset,
  };
}

export function RecipeEditDialog({ recipe, existingIds, onSave, onCancel }: Props) {
  const isNew = recipe === null;
  // archives는 order 기준으로 한 번 정렬해서 시작 — 카드 목록의 배열 위치가 곧
  // 실행 순서라는 불변식을 처음부터 보장(그 뒤로는 renumberArchiveOrders가 유지).
  const [draft, setDraft] = useState<Recipe>(() => {
    const initial = recipe ?? defaultNewRecipe();
    return { ...initial, archives: [...initial.archives].sort((a, b) => a.order - b.order) };
  });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<RecipeEditTab>("basic");
  // 목록(카드/트리)은 식별만, 편집 폼은 선택된 항목 하나에 대해서만 그 밑에 로드 —
  // 항목마다 폼 전체를 반복 렌더링하지 않기 위함.
  const [selectedArchiveIndex, setSelectedArchiveIndex] = useState<number | null>(() =>
    draft.archives.length > 0 ? 0 : null,
  );
  const [selectedFileIndex, setSelectedFileIndex] = useState<number | null>(null);
  // 목적지 트리에서 파일 선택과 폴더 선택은 서로 배타적 — 아래 편집 폼 자리 하나를
  // 공유하므로 한쪽을 고르면 다른 쪽은 비운다.
  const [selectedFolderPath, setSelectedFolderPath] = useState<string | null>(null);
  const selectFile = (index: number) => {
    setSelectedFileIndex(index);
    setSelectedFolderPath(null);
  };
  const selectFolder = (path: string) => {
    setSelectedFolderPath(path);
    setSelectedFileIndex(null);
  };
  // 폴더 옵션(FolderRuleMode) 복사/붙여넣기 — 여러 폴더에 같은 규칙을 반복 입력하지
  // 않도록. 선택된 폴더가 바뀌어도(심지어 잠깐 파일을 선택했다 돌아와도) 유지되도록
  // 부모(이 컴포넌트)가 들고 있는다 — `FolderRuleEditor` 로컬 state 면 그 폴더를
  // 벗어나는 순간(트리에서 다른 폴더/파일 선택) 다시 열릴 때 초기화될 위험이 있다.
  const [copiedFolderRule, setCopiedFolderRule] = useState<FolderRuleMode | null>(null);
  const [addingArchive, setAddingArchive] = useState(false);
  const [importingFolder, setImportingFolder] = useState(false);

  // folder_rules 에 등록된 경로는 파일이 0개여도 폴더 노드로 남는다 — "폴더 비우기"가
  // 빈 규칙을 걸어 폴더를 남기는 방식(아래 handleEmptyFolder)이 여기서 완성된다.
  const fileTree = useMemo(
    () => buildFileTree(draft.files, draft.folder_rules.map((r) => r.path)),
    [draft.files, draft.folder_rules],
  );

  const handleFolderImport = (paths: string[]) => {
    setDraft((prev) => {
      const existingPaths = new Set(prev.files.map((f) => f.path));
      const additions: RecipeFile[] = paths
        .filter((p) => !existingPaths.has(p))
        .map((p) => ({ path: p, override_content: null }));
      return { ...prev, files: [...prev.files, ...additions] };
    });
  };

  /** `mode`가 `null`이면 규칙 삭제(기본 화이트리스트로 복귀), 아니면 그 폴더의 규칙을
   * 새로 설정하거나 교체 — `Recipe.folder_rules` 안에서 `path`는 유일해야 하므로 항상
   * 기존 항목을 지우고 다시 넣는다. */
  const handleFolderRuleChange = (path: string, mode: FolderRuleMode | null) => {
    setDraft((prev) => {
      const rest = prev.folder_rules.filter((r) => r.path !== path);
      return { ...prev, folder_rules: mode ? [...rest, { path, mode }] : rest };
    });
  };

  /** 이미 압축을 풀어본 폴더(또는 이미 설치된 인스턴스 폴더)를 골라서, 그 안의 파일
   * 전부를 화이트리스트(`Recipe.files`)에 자동으로 채워넣는다 — 수백 개를 손으로
   * 타이핑하지 않기 위함. 화이트리스트 정책 자체(예외 없이 항상 강제)는 완화하지
   * 않고, 작성 부담만 도구로 상쇄한다. 고른 폴더 자체(예: `SomeApp`)는 목적지 루트와
   * 동일시돼 결과 경로에서 벗겨진다 — "폴더=루트" 전제는 그대로 유지(사용자 확인).
   * 특정 목적지 폴더 밑으로 불러오고 싶으면 `handleImportFolderAt`(아래)을 쓴다. */
  const handleImportFolder = async () => {
    setError(null);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ directory: true, multiple: false, title: "화이트리스트로 채울 폴더 선택" });
      if (!picked || typeof picked !== "string") return;
      setImportingFolder(true);
      const paths = await scanFolderRelativePaths(picked);
      handleFolderImport(paths);
    } catch (e) {
      setError(String(e));
    } finally {
      setImportingFolder(false);
    }
  };

  /** `handleImportFolder`의 "선택된 폴더 밑으로" 버전 — 고른 OS 폴더의 내용물 전부를
   * `destPath`(트리에서 선택된 폴더의 경로) 접두사를 붙여 추가한다. 예: `destPath`가
   * "config"이고 고른 폴더 안에 "settings.json"이 있으면 "config/settings.json"으로
   * 들어간다 — 고른 폴더 자체의 이름은 여기서도 여전히 벗겨진다(내용물만 옮겨 붙이는
   * 것이지, 이름째로 복제하는 게 아님). */
  const handleImportFolderAt = async (destPath: string) => {
    setError(null);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ directory: true, multiple: false, title: `"${destPath}" 밑으로 불러올 폴더 선택` });
      if (!picked || typeof picked !== "string") return;
      setImportingFolder(true);
      const paths = await scanFolderRelativePaths(picked);
      handleFolderImport(paths.map((p) => `${destPath}/${p}`));
    } catch (e) {
      setError(String(e));
    } finally {
      setImportingFolder(false);
    }
  };

  const handleAddFileAt = (folderPath: string) => {
    const newFile: RecipeFile = { path: folderPath ? `${folderPath}/` : "", override_content: null };
    setDraft((prev) => ({ ...prev, files: [...prev.files, newFile] }));
    selectFile(draft.files.length);
  };

  const handleRemoveFile = (index: number) => {
    setDraft((prev) => ({ ...prev, files: prev.files.filter((_, i) => i !== index) }));
    setSelectedFileIndex((cur) => {
      if (cur === null || cur === index) return null;
      return cur > index ? cur - 1 : cur;
    });
  };

  /** "폴더 비우기"(선택된 폴더 편집 패널) — 그 아래 파일만 지우고 선택 상태는 그대로
   * 둔다. 폴더 자체는 파일 경로 접두사로만 존재하는 파생 구조라(`buildFileTree`),
   * 파일을 다 지우면 뭔가 다른 근거가 없는 한 트리에서도 사라진다 — 그래서 그 경로에
   * `folder_rules` 항목이 아직 없으면 "전체 허용"(`Passthrough`) 규칙을 하나 걸어
   * 폴더가 계속 보이게 한다. 나중에 이 폴더를 다시 채울 걸 전제로 비우는 동작이라,
   * 채워 넣을 때마다 화이트리스트에 일일이 등록하지 않아도 되는 쪽을 기본으로 둔다.
   * 이미 규칙이 있으면 손대지 않는다. "폴더 제거"(트리 행의 ✕, 아래)와 다른 점은 이
   * 규칙 유지뿐 — 나중에 다시 파일을 채울 폴더는 비우기, 아예 없앨 폴더는 제거를 쓴다. */
  const handleEmptyFolder = (path: string) => {
    const prefix = `${path}/`;
    const isUnderPath = (p: string) => p === path || p.startsWith(prefix);
    const selectedPath =
      selectedFileIndex !== null ? (draft.files[selectedFileIndex]?.path ?? null) : null;
    // `newFiles`는 이 핸들러 안에서 딱 한 번만 draft를 건드리므로(다른 setDraft와
    // 같은 이벤트에서 안 겹침) `draft`에서 직접 계산해도 안전 — setState 업데이터가
    // 언제 실제로 실행되는지(React가 보장 안 함)에 기대지 않는다.
    const newFiles = draft.files.filter((f) => !isUnderPath(f.path));
    const hasRule = draft.folder_rules.some((r) => r.path === path);
    setDraft((prev) => ({
      ...prev,
      files: prev.files.filter((f) => !isUnderPath(f.path)),
      folder_rules: hasRule
        ? prev.folder_rules
        : [...prev.folder_rules, { path, mode: { kind: "passthrough" } }],
    }));
    if (selectedPath !== null && !isUnderPath(selectedPath)) {
      const newIndex = newFiles.findIndex((f) => f.path === selectedPath);
      setSelectedFileIndex(newIndex >= 0 ? newIndex : null);
    } else {
      setSelectedFileIndex(null);
    }
  };

  /** 폴더는 별도 엔티티가 아니라 `draft.files`의 경로 접두사로만 존재하는 파생
   * 구조(`buildFileTree` 참고)라, "폴더 제거"는 실제로는 그 접두사 아래 파일 전부를
   * 한 번에 제거하는 것과 같다. 그 폴더에 걸린 `folder_rules`도 이제 대상이 사라지니
   * 같이 정리 — 안 지우면 저장된 규칙이 가리키는 폴더가 없는 채로 남는다. */
  const handleRemoveFolder = (path: string) => {
    const prefix = `${path}/`;
    const isUnderRemoved = (p: string) => p === path || p.startsWith(prefix);
    const selectedPath =
      selectedFileIndex !== null ? (draft.files[selectedFileIndex]?.path ?? null) : null;
    const newFiles = draft.files.filter((f) => !isUnderRemoved(f.path));
    setDraft((prev) => ({
      ...prev,
      files: prev.files.filter((f) => !isUnderRemoved(f.path)),
      folder_rules: prev.folder_rules.filter((r) => !isUnderRemoved(r.path)),
    }));
    if (selectedPath !== null && !isUnderRemoved(selectedPath)) {
      const newIndex = newFiles.findIndex((f) => f.path === selectedPath);
      setSelectedFileIndex(newIndex >= 0 ? newIndex : null);
    } else {
      setSelectedFileIndex(null);
    }
    setSelectedFolderPath((cur) => (cur !== null && isUnderRemoved(cur) ? null : cur));
  };

  // 압축을 등록하려면 애초에 컴퓨터에서 그 파일을 찾아야 한다 — URL을 먼저 적고
  // 해시를 나중에 채우는 순서는 실제 작업 순서와 안 맞는다. 그래서 "추가" 자체가
  // 파일 선택으로 시작한다: 고르면 그 파일의 해시가 채워진 압축이 생기고, 취소하면
  // 아무것도 추가되지 않는다(URL만 있고 해시 없는 압축이 생기는 경로를 아예 없앰).
  const handleAddArchive = async () => {
    setError(null);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ multiple: false, title: "압축 파일 선택" });
      if (!picked || typeof picked !== "string") return;
      setAddingArchive(true);
      const hash = await computeFileSha256(picked);
      // 새 압축은 항상 배열 끝에 추가되므로, 지금(await 이후) 시점의 `draft.archives.length`가
      // 곧 그 새 압축의 인덱스다 — 파일 선택 다이얼로그가 모달이라 그 사이 다른 압축
      // 추가/삭제가 끼어들 수 없음을 전제로 한다(setState 업데이터 내부 값을 밖에서
      // 동기적으로 읽으려 했던 이전 버전의 버그 패턴을 피함).
      const archive: ArchiveExtraction = {
        ...defaultArchive(draft.archives),
        verification: { kind: "sha256", hash },
      };
      const newIndex = draft.archives.length;
      setDraft((prev) => ({ ...prev, archives: renumberArchiveOrders([...prev.archives, archive]) }));
      setSelectedArchiveIndex(newIndex);
    } catch (e) {
      setError(String(e));
    } finally {
      setAddingArchive(false);
    }
  };

  // 압축 하나 = 옵션 하나(2026-08 재설계, ArchiveOptionalField 참고) — 압축을
  // "선택 항목"에서 체크 해제할 땐 그 압축이 소유한 optional_group도 같이 지워지는데,
  // 압축 자체를 통째로 제거할 땐 그 정리를 거치지 않아 주인 없는 optional_group이
  // "설치할 구성 요소" 목록에 유령으로 계속 남아있던 버그. 다른 압축/파일이 여전히
  // 같은 그룹을 참조 중이면(정상 설계상 없어야 하지만 방어적으로) 지우지 않는다.
  const handleRemoveArchive = (index: number) => {
    const removedGroup = draft.archives[index]?.optional_group ?? null;
    setDraft((prev) => {
      const archives = renumberArchiveOrders(prev.archives.filter((_, i) => i !== index));
      const stillReferenced =
        removedGroup !== null &&
        (archives.some((a) => a.optional_group === removedGroup) ||
          prev.files.some((f) => f.optional_group === removedGroup));
      const optional_groups =
        removedGroup !== null && !stillReferenced
          ? prev.optional_groups.filter((g) => g.id !== removedGroup)
          : prev.optional_groups;
      return { ...prev, archives, optional_groups };
    });
    setSelectedArchiveIndex((cur) => {
      if (cur === null || cur === index) return null;
      return cur > index ? cur - 1 : cur;
    });
  };

  // 카드를 드래그해서 옮기면 그 배열 위치가 곧 실행 순서 — `order` 필드를 손으로
  // 입력하게 두지 않고 항상 위치에서 다시 계산해 덮어쓴다.
  const handleReorderArchive = (from: number, to: number) => {
    if (from === to) return;
    setDraft((prev) => ({ ...prev, archives: renumberArchiveOrders(moveItem(prev.archives, from, to)) }));
    setSelectedArchiveIndex((cur) => (cur === null ? null : adjustSelectedIndexAfterMove(cur, from, to)));
  };

  /** "압축 다운로드" 탭의 옵션 드롭다운에서 "+ 새 옵션 만들기"로 즉석 생성 — 이름만
   * 받아 id는 `Recipe.id`와 같은 규칙(slugify + 중복 회피)으로 자동 생성한다. 반환값을
   * 호출자가 그 자리에서 바로 `optional_group`에 대입해 생성과 배정을 한 동작으로
   * 묶는다(빈 이름의 미완성 그룹이 잠깐이라도 목록에 떠 있지 않도록). */
  const handleCreateOptionalGroup = (label: string): string => {
    const trimmed = label.trim();
    if (!trimmed) return "";
    // `draft`에서 직접 계산 — 이 핸들러 안에서 draft를 건드리는 setState 호출은
    // 이거 하나뿐이라 안전하다(React가 setState 업데이터를 언제 실제로 실행하는지는
    // 보장하지 않으므로, 그 안에서 바깥 변수에 값을 대입하고 곧바로 읽어오는 패턴에
    // 기대면 안 됨 — 이전에 이 함수가 그 패턴으로 인해 반환값이 항상 빈 문자열이 돼
    // "선택 항목" 체크박스가 안 먹는 버그가 났었음).
    const id = uniqueId(slugify(trimmed), draft.optional_groups.map((g) => g.id));
    setDraft((prev) => ({
      ...prev,
      optional_groups: [...prev.optional_groups, { id, label: trimmed, default_selected: false }],
    }));
    return id;
  };

  const handleUpdateOptionalGroup = (id: string, patch: Partial<OptionalGroup>) => {
    setDraft((prev) => ({
      ...prev,
      optional_groups: prev.optional_groups.map((g) => (g.id === id ? { ...g, ...patch } : g)),
    }));
  };

  /** 옵션 자체를 지운다 — 그 옵션을 참조하던 압축/파일의 배정도 같이 풀어준다(그대로
   * 두면 목록에서 사라진 이름을 가리키는 유령 참조가 남아, 그 압축/파일이 영원히
   * "선택 안 됨" 취급되는 조용한 버그가 됨). */
  const handleDeleteOptionalGroup = (id: string) => {
    setDraft((prev) => ({
      ...prev,
      optional_groups: prev.optional_groups.filter((g) => g.id !== id),
      archives: prev.archives.map((a) => (a.optional_group === id ? { ...a, optional_group: null } : a)),
      files: prev.files.map((f) => (f.optional_group === id ? { ...f, optional_group: null } : f)),
    }));
  };

  const handleSave = async () => {
    if (isNew && draft.name.trim().length === 0) {
      setError("이름을 입력하세요.");
      return;
    }
    if (draft.archives.length === 0 && draft.files.length === 0) {
      setError("압축 다운로드 또는 파일이 최소 1개 필요합니다.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const toSave = isNew
        ? { ...draft, id: uniqueId(slugify(draft.name), existingIds) }
        : draft;
      await onSave(toSave);
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
      aria-labelledby="recipe-edit-title"
      onClick={() => !saving && onCancel()}
    >
      <div
        className="flex h-[85vh] w-full max-w-3xl flex-col rounded-lg border border-neutral-800 bg-neutral-900 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 id="recipe-edit-title" className="px-6 py-3 text-base font-semibold text-neutral-50">
          {isNew ? "새 앱 추가" : "앱 편집"}
        </h3>

        <div className="flex min-h-0 flex-1 border-t border-neutral-800">
          <nav className="w-36 shrink-0 space-y-1 overflow-y-auto border-r border-neutral-800 p-3">
            {TAB_ORDER.map((t) => (
              <button
                key={t}
                type="button"
                onClick={() => setTab(t)}
                className={`w-full cursor-pointer rounded px-2 py-1.5 text-left text-xs ${
                  tab === t
                    ? "bg-neutral-800 text-neutral-100"
                    : "text-neutral-400 hover:bg-neutral-800/50 hover:text-neutral-200"
                }`}
              >
                {TAB_LABELS[t]}
              </button>
            ))}
          </nav>

          <div className="flex min-h-0 flex-1 flex-col p-6">
              {tab === "basic" && (
                <div className="min-h-0 flex-1 space-y-4 overflow-y-auto">
                  <Field label="이름">
                    <TextInput value={draft.name} onChange={(name) => setDraft((prev) => ({ ...prev, name }))} />
                  </Field>
                  <Field
                    label="아이콘 URL"
                    action={<ImageUrlWarning url={draft.recipe_info.icon_url ?? ""} />}
                  >
                    <TextInput
                      value={draft.recipe_info.icon_url ?? ""}
                      onChange={(v) =>
                        setDraft((prev) => ({ ...prev, recipe_info: { ...prev.recipe_info, icon_url: v || null } }))
                      }
                    />
                  </Field>
                  <Field
                    label="배경 URL"
                    action={<ImageUrlWarning url={draft.recipe_info.background_url ?? ""} />}
                  >
                    <TextInput
                      value={draft.recipe_info.background_url ?? ""}
                      onChange={(v) =>
                        setDraft((prev) => ({
                          ...prev,
                          recipe_info: { ...prev.recipe_info, background_url: v || null },
                        }))
                      }
                    />
                  </Field>
                </div>
              )}

              {tab === "archives" && (
                <div className="flex min-h-0 flex-1 flex-col gap-3">
                  <Field
                    label="압축 다운로드"
                    className="shrink-0"
                    action={
                      <button
                        type="button"
                        disabled={addingArchive}
                        onClick={() => void handleAddArchive()}
                        className="shrink-0 cursor-pointer text-xs text-neutral-400 hover:text-neutral-200 disabled:opacity-50"
                      >
                        {addingArchive ? "압축 파일 확인 중..." : "+ 압축 추가"}
                      </button>
                    }
                  >
                    <ArchiveCardList
                      archives={draft.archives}
                      optionalGroups={draft.optional_groups}
                      selectedIndex={selectedArchiveIndex}
                      onSelect={setSelectedArchiveIndex}
                      onRemove={handleRemoveArchive}
                      onReorder={handleReorderArchive}
                    />
                  </Field>
                  <div className="min-h-0 flex-1 overflow-y-auto">
                    {selectedArchiveIndex !== null && draft.archives[selectedArchiveIndex] ? (
                      <ArchiveEditor
                        archive={draft.archives[selectedArchiveIndex]}
                        optionalGroups={draft.optional_groups}
                        files={draft.files}
                        folderRules={draft.folder_rules}
                        onChange={(a) =>
                          setDraft((prev) => ({
                            ...prev,
                            archives: prev.archives.map((it, i) => (i === selectedArchiveIndex ? a : it)),
                          }))
                        }
                        onCreateOptionalGroup={handleCreateOptionalGroup}
                        onUpdateOptionalGroup={handleUpdateOptionalGroup}
                        onDeleteOptionalGroup={handleDeleteOptionalGroup}
                      />
                    ) : (
                      <p className="text-xs text-neutral-500">
                        위에서 압축을 선택하거나 추가하면 여기에 편집 폼이 나타납니다.
                      </p>
                    )}
                  </div>
                </div>
              )}

              {tab === "files" && (
                <div className="flex min-h-0 flex-1 flex-col gap-3">
                  <Field
                    label="목적지 트리"
                    className="shrink-0"
                    action={
                      <button
                        type="button"
                        disabled={importingFolder}
                        onClick={() => void handleImportFolder()}
                        className="shrink-0 cursor-pointer text-xs text-neutral-400 hover:text-neutral-200 disabled:opacity-50"
                      >
                        {importingFolder ? "불러오는 중..." : "+ 폴더 불러오기"}
                      </button>
                    }
                  >
                    <FileTreeView
                      root={fileTree}
                      files={draft.files}
                      optionalGroups={draft.optional_groups}
                      selectedIndex={selectedFileIndex}
                      onSelect={selectFile}
                      onAddAt={handleAddFileAt}
                      onRemove={handleRemoveFile}
                      onRemoveFolder={handleRemoveFolder}
                      folderRules={draft.folder_rules}
                      selectedFolderPath={selectedFolderPath}
                      onSelectFolder={selectFolder}
                    />
                  </Field>
                  <div className="min-h-0 flex-1 overflow-y-auto">
                    {selectedFileIndex !== null && draft.files[selectedFileIndex] ? (
                      <Field label={`선택된 파일: ${draft.files[selectedFileIndex].path || "(경로 없음)"}`}>
                        <RecipeFileEditor
                          file={draft.files[selectedFileIndex]}
                          onChange={(f) =>
                            setDraft((prev) => ({
                              ...prev,
                              files: prev.files.map((it, i) => (i === selectedFileIndex ? f : it)),
                            }))
                          }
                          onRemove={() => handleRemoveFile(selectedFileIndex)}
                        />
                      </Field>
                    ) : selectedFolderPath !== null ? (
                      <Field
                        label={`선택된 폴더: ${selectedFolderPath}`}
                        action={
                          <div className="flex shrink-0 items-center gap-2">
                            <button
                              type="button"
                              disabled={importingFolder}
                              onClick={() => void handleImportFolderAt(selectedFolderPath)}
                              className="cursor-pointer text-xs text-neutral-400 hover:text-neutral-200 disabled:opacity-50"
                            >
                              {importingFolder ? "불러오는 중..." : "여기로 폴더 불러오기"}
                            </button>
                            <button
                              type="button"
                              onClick={() => handleEmptyFolder(selectedFolderPath)}
                              className="cursor-pointer text-xs text-neutral-400 hover:text-neutral-200"
                            >
                              폴더 비우기
                            </button>
                          </div>
                        }
                      >
                        <FolderRuleEditor
                          mode={draft.folder_rules.find((r) => r.path === selectedFolderPath)?.mode ?? null}
                          onChange={(mode) => handleFolderRuleChange(selectedFolderPath, mode)}
                          copiedRule={copiedFolderRule}
                          onCopy={setCopiedFolderRule}
                        />
                      </Field>
                    ) : (
                      <p className="text-xs text-neutral-500">
                        트리에서 파일이나 폴더를 선택하면 여기에 편집 폼이 나타납니다.
                      </p>
                    )}
                  </div>
                </div>
              )}

              {tab === "launch" && (
                <div className="min-h-0 flex-1 space-y-4 overflow-y-auto">
                  <Field label="실행 방식">
                    <Select
                      value={draft.launch.kind}
                      onChange={(kind) =>
                        setDraft((prev) => ({ ...prev, launch: defaultLaunchAction(kind as LaunchActionKind) }))
                      }
                      options={(Object.keys(LAUNCH_KIND_LABELS) as LaunchActionKind[]).map((k) => ({
                        value: k,
                        label: LAUNCH_KIND_LABELS[k],
                      }))}
                    />
                  </Field>
                  <LaunchActionFields
                    launch={draft.launch}
                    files={draft.files}
                    folderRules={draft.folder_rules}
                    onChange={(launch) => setDraft((prev) => ({ ...prev, launch }))}
                  />
                </div>
              )}
          </div>
        </div>

        <div className="border-t border-neutral-800 px-6 py-4">
          {error && (
            <p className="mb-3 break-all rounded border border-red-900/50 bg-red-950/30 p-2 text-xs text-red-200">
              {error}
            </p>
          )}
          <div className="flex justify-end gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={saving}
              onClick={onCancel}
              className="cursor-pointer"
            >
              취소
            </Button>
            <Button
              size="sm"
              disabled={saving}
              onClick={() => void handleSave()}
              className="min-w-[80px] cursor-pointer"
            >
              {saving ? "저장 중..." : "저장"}
            </Button>
          </div>
        </div>
      </div>
    </div>
    </Portal>
  );
}

function defaultSha256(): ArtifactVerification {
  return { kind: "sha256", hash: "" };
}

/** 새 압축의 기본 order — 기존 압축들 중 최댓값+1(항상 유일한 값이 되도록). 평소엔
 * 신경 쓸 필요 없고, 겹치는 목적지의 실행 순서를 일부러 바꾸고 싶을 때만 직접 수정. */
function defaultArchive(existing: ArchiveExtraction[]): ArchiveExtraction {
  return {
    url: "",
    label: null,
    verification: defaultSha256(),
    order: existing.length + 1,
    extract_to: "",
    optional_group: null,
  };
}

/** `order`는 이제 사용자가 손으로 적는 값이 아니라 카드 목록의 실제 배열 위치에서
 * 매번 다시 계산해 덮어쓰는 값 — 드래그로 카드를 옮기면 곧바로 이 함수가 전체
 * 목록의 `order`를 1부터 다시 매긴다(백엔드는 여전히 유일한 값만 요구하지만,
 * 항상 위치와 정확히 일치시켜 혼란을 없앤다). */
function renumberArchiveOrders(archives: ArchiveExtraction[]): ArchiveExtraction[] {
  return archives.map((a, i) => ({ ...a, order: i + 1 }));
}

function defaultLaunchAction(kind: LaunchActionKind): LaunchAction {
  switch (kind) {
    case "spawn_process":
      return { kind: "spawn_process", entry_point: "", entry_args: [] };
    case "third_party_app_launch":
      // 특정 app_id 를 미리 정하지 않는다 — `LaunchActionFields`가 등록된 목록을
      // 조회한 뒤 첫 항목으로 채운다(어떤 앱이 "기본"인지는 목록이 정할 뿐, 코드가
      // 정하지 않는다).
      return { kind: "third_party_app_launch", app_id: "" };
  }
}

// ---------------------------------------------------------------------------
// archives 편집
// ---------------------------------------------------------------------------

/** 카드 목록에서 압축을 식별할 때 쓰는 짧은 이름 — `label`을 직접 지정했으면 그걸
 * 그대로 쓰고, 없으면 URL 마지막 경로 세그먼트에서 유도한다. 단축 URL 서비스 등은
 * 그 세그먼트가 알아볼 수 없는 문자열이라 `label`로 사람이 읽을 이름을 따로 지정할
 * 수 있다(2026-08, 사용자 확인). URL이 아직 안 써졌거나 파싱 안 되면(작성 중) 원본
 * 문자열/기본값으로 폴백. */
function archiveDisplayName(url: string, label?: string | null): string {
  const trimmedLabel = label?.trim();
  if (trimmedLabel) return trimmedLabel;
  const trimmed = url.trim();
  if (!trimmed) return "새 압축";
  const lastSegment = (s: string) => s.split("/").filter((seg) => seg.length > 0).pop();
  try {
    const last = lastSegment(new URL(trimmed).pathname);
    if (last) return decodeURIComponent(last);
  } catch {
    // URL 파싱 실패(아직 온전한 URL이 아닌 작성 중 값) — 원본 문자열에서 폴백 추출.
  }
  const last = lastSegment(trimmed);
  return last ? decodeURIComponent(last) : trimmed;
}

/** 압축 목록 — 카드는 식별 정보(순서·이름·뱃지)+삭제(✕)만 갖고, 편집은 선택된
 * 카드 하나에 대해서만 `ArchiveEditor` 가 그 아래에 한 번 렌더링됨(호출부 참고) —
 * 압축이 여러 개여도 폼이 반복되지 않도록. 세로 스크롤(카드가 늘어나는 방향과
 * 트리 뷰 등 다른 목록 UI들과 일관되게). 드래그로 순서를 바꾼다 — order 필드를
 * 손으로 입력하는 대신, 카드 위치 자체가 실행 순서(호출부의 `renumberArchiveOrders`
 * 참고). */
function ArchiveCardList({
  archives,
  optionalGroups,
  selectedIndex,
  onSelect,
  onRemove,
  onReorder,
}: {
  archives: ArchiveExtraction[];
  optionalGroups: OptionalGroup[];
  selectedIndex: number | null;
  onSelect: (index: number) => void;
  onRemove: (index: number) => void;
  onReorder: (from: number, to: number) => void;
}) {
  const drag = useDragReorder(onReorder);

  return (
    <div
      ref={drag.containerRef}
      onDragOver={drag.handleContainerDragOver}
      onDrop={drag.handleDrop}
      className="max-h-[20vh] space-y-2 overflow-y-auto rounded border border-neutral-800 bg-neutral-950/40 p-2"
    >
      {archives.length === 0 && (
        <p className="px-1 py-2 text-xs text-neutral-500">아직 압축이 없습니다.</p>
      )}
      {archives.map((a, i) => (
        <div key={i}>
          <DropGapIndicator show={drag.dragIndex !== null && drag.gapIndex === i} />
          <div
            draggable
            onDragStart={() => drag.handleDragStart(i)}
            onDragOver={(e) => drag.handleCardDragOver(i, e)}
            onDragEnd={drag.handleDragEnd}
            onClick={() => onSelect(i)}
            className={`group relative select-none cursor-grab rounded border p-2 text-xs active:cursor-grabbing ${
              selectedIndex === i
                ? "border-neutral-600 bg-neutral-800 text-neutral-100"
                : "border-neutral-800 bg-neutral-900 text-neutral-400 hover:border-neutral-700 hover:text-neutral-200"
            } ${drag.dragIndex === i ? "opacity-40" : ""}`}
          >
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                onRemove(i);
              }}
              className="absolute right-1 top-1/2 hidden -translate-y-1/2 cursor-pointer text-red-300 hover:text-red-200 group-hover:inline"
              aria-label="삭제"
            >
              ✕
            </button>
            <div className="flex items-center justify-between gap-2 pr-4">
              <div className="flex min-w-0 items-center gap-1.5">
                <span className="shrink-0 rounded bg-white px-1.5 py-0.5 text-[10px] font-medium text-black">
                  {a.order}
                </span>
                <p className="min-w-0 truncate font-medium text-neutral-200">{archiveDisplayName(a.url, a.label)}</p>
              </div>
              <div className="flex shrink-0 items-center gap-1 text-[10px] text-neutral-500">
                {(a.path_overrides?.length ?? 0) > 0 && (
                  <span className="rounded bg-white px-1 text-black">재배치 {a.path_overrides!.length}</span>
                )}
                {a.optional_group && (
                  <span className="rounded bg-white px-1 text-black">
                    {optionalGroups.find((g) => g.id === a.optional_group)?.label ?? a.optional_group}
                  </span>
                )}
              </div>
            </div>
          </div>
        </div>
      ))}
      <DropGapIndicator show={drag.dragIndex !== null && drag.gapIndex === archives.length} />
    </div>
  );
}

/** 드래그 재정렬 중 "여기에 놓임"을 보여주는 가는 막대 — 실제 공간을 차지하지
 * 않다가(`show`가 false면 렌더링 자체를 생략) 드래그 중에만 나타난다. */
function DropGapIndicator({ show }: { show: boolean }) {
  if (!show) return null;
  return <div className="-my-1 h-0.5 rounded bg-neutral-300" />;
}

function ArchiveEditor({
  archive,
  optionalGroups,
  files,
  folderRules,
  onChange,
  onCreateOptionalGroup,
  onUpdateOptionalGroup,
  onDeleteOptionalGroup,
}: {
  archive: ArchiveExtraction;
  optionalGroups: OptionalGroup[];
  files: RecipeFile[];
  folderRules: FolderRule[];
  onChange: (a: ArchiveExtraction) => void;
  onCreateOptionalGroup: (label: string) => string;
  onUpdateOptionalGroup: (id: string, patch: Partial<OptionalGroup>) => void;
  onDeleteOptionalGroup: (id: string) => void;
}) {
  return (
    <div className="space-y-4 rounded border border-neutral-800 bg-neutral-950/40 p-3">
      <Field label="제목 (선택 — 카드 목록에 표시할 이름, 비우면 URL에서 유도)">
        <TextInput
          value={archive.label ?? ""}
          placeholder="예: 스킨 팩 (단축 URL처럼 주소만으론 못 알아볼 때)"
          onChange={(label) => onChange({ ...archive, label: label || null })}
        />
      </Field>
      <Field label="다운로드 소스">
        <div className="space-y-2">
          <TextInput
            value={archive.url}
            placeholder="다운로드 URL (직접 링크 아니어도 자동 인식)"
            onChange={(url) => onChange({ ...archive, url })}
          />
        </div>
      </Field>

      <Field label="무결성 검증">
        <ArtifactVerificationFields onChange={(verification) => onChange({ ...archive, verification })} />
      </Field>

      <Field label="설치 위치">
        <div className="space-y-2">
          <div className="flex items-center gap-2">
            <div className="min-w-0 flex-1">
              <TextInput
                value={archive.extract_to}
                placeholder="해제 위치"
                readOnly
                onChange={(extract_to) => onChange({ ...archive, extract_to })}
              />
            </div>
            <DestinationPathPicker
              files={files}
              mode="folder"
              onPick={(extract_to) => onChange({ ...archive, extract_to })}
              extraFolderPaths={folderRules.map((r) => r.path)}
            />
          </div>
        </div>
      </Field>

      <Field label="설치 옵션">
        <ArchiveOptionalField
          archive={archive}
          optionalGroups={optionalGroups}
          onChange={onChange}
          onCreateOptionalGroup={onCreateOptionalGroup}
          onUpdateOptionalGroup={onUpdateOptionalGroup}
          onDeleteOptionalGroup={onDeleteOptionalGroup}
        />
      </Field>

      <Field label="파일/폴더 재배치">
        <ListEditor
          items={archive.path_overrides ?? []}
          onChange={(path_overrides) => onChange({ ...archive, path_overrides })}
          newItem={defaultPathOverride}
          addLabel="+ 재배치 규칙 추가"
          renderItem={(override, update, remove) => (
            <PathOverrideEditor override={override} files={files} onChange={update} onRemove={remove} />
          )}
        />
      </Field>
    </div>
  );
}

/** 압축 하나가 곧 옵션 하나 — 다른 압축이 만든 옵션을 골라 쓰는 공유 풀이 아니다
 * (2026-08, 사용자 확인: "설치 옵션들을 별도로 만드는게 아니고, 하나의 압축파일이
 * 하나의 옵션을 담당하게 해야지"). 체크하면 이 압축 전용 [`OptionalGroup`]을 그
 * 자리에서 만들어 이름/기본선택을 바로 편집하고, 체크 해제하면 그 옵션을 지운다 —
 * 목록에서 고르는 UI 자체가 없다. */
function ArchiveOptionalField({
  archive,
  optionalGroups,
  onChange,
  onCreateOptionalGroup,
  onUpdateOptionalGroup,
  onDeleteOptionalGroup,
}: {
  archive: ArchiveExtraction;
  optionalGroups: OptionalGroup[];
  onChange: (a: ArchiveExtraction) => void;
  onCreateOptionalGroup: (label: string) => string;
  onUpdateOptionalGroup: (id: string, patch: Partial<OptionalGroup>) => void;
  onDeleteOptionalGroup: (id: string) => void;
}) {
  const group = optionalGroups.find((g) => g.id === archive.optional_group) ?? null;

  const handleToggle = (checked: boolean) => {
    if (checked) {
      const id = onCreateOptionalGroup("새 옵션");
      onChange({ ...archive, optional_group: id });
    } else if (archive.optional_group) {
      onDeleteOptionalGroup(archive.optional_group);
      onChange({ ...archive, optional_group: null });
    }
  };

  return (
    <div className="space-y-2">
      <label className="flex cursor-pointer items-center gap-2 text-xs text-neutral-400">
        <input type="checkbox" checked={!!group} onChange={(e) => handleToggle(e.target.checked)} />
        선택 항목 (사용자가 설치 시 고를 때만 받음 — 체크 안 하면 항상 필수)
      </label>
      {group && (
        <div className="space-y-2 rounded border border-neutral-800 bg-neutral-950/40 p-2">
          <TextInput
            value={group.label}
            placeholder="옵션 이름"
            onChange={(label) => onUpdateOptionalGroup(group.id, { label })}
          />
          <label className="flex cursor-pointer items-center gap-2 text-xs text-neutral-400">
            <input
              type="checkbox"
              checked={group.default_selected}
              onChange={(e) => onUpdateOptionalGroup(group.id, { default_selected: e.target.checked })}
            />
            설치 시 기본 선택
          </label>
        </div>
      )}
    </div>
  );
}

function defaultPathOverride(): PathOverride {
  return { from: "", to: "" };
}

function PathOverrideEditor({
  override,
  files,
  onChange,
  onRemove,
}: {
  override: PathOverride;
  files: RecipeFile[];
  onChange: (o: PathOverride) => void;
  onRemove: () => void;
}) {
  return (
    <div className="flex items-center gap-2">
      <div className="min-w-0 flex-1">
        <TextInput
          value={override.from}
          placeholder="압축 안 경로 — 파일 하나(Playing1.opi) 또는 폴더 통째(A)"
          onChange={(from) => onChange({ ...override, from })}
        />
      </div>
      <div className="min-w-0 flex-1">
        <TextInput
          value={override.to}
          placeholder="보낼 위치 (폴더면 그 밑 구조 유지, 비우면 루트)"
          readOnly
          onChange={(to) => onChange({ ...override, to })}
        />
      </div>
      <DestinationPathPicker files={files} mode="file" onPick={(to) => onChange({ ...override, to })} />
      <RemoveButton onClick={onRemove} />
    </div>
  );
}

/** 압축의 무결성 해시는 실제 파일에서 계산한 값만 유효하다 — 손으로 타이핑하면
 * 오타가 나기 쉽고(실제로 이 프로젝트에서 오타로 인한 설치 버그가 있었음), 애초에
 * "이 압축이 어떤 파일인지"를 사람이 손으로 옮겨적을 이유가 없다. 그래서 해시
 * 문자열 자체는 편집 폼에 노출하지 않고, 파일을 고르면 그 파일의 실제 해시로
 * 바로 교체하는 동작만 제공한다 — 버튼 이름이 "해시 계산"이 아니라 "압축 파일
 * 변경"인 이유(무엇을 계산하는지가 아니라 무엇을 바꾸는지가 본질). 값이 틀리면
 * 저장 시점 검증(백엔드) 또는 설치 시점 해시 대조에서 에러로 드러난다. */
function ArtifactVerificationFields({ onChange }: { onChange: (v: ArtifactVerification) => void }) {
  const [computing, setComputing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleChangeFile = async () => {
    setError(null);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ multiple: false, title: "압축 파일 선택" });
      if (!picked || typeof picked !== "string") return;
      setComputing(true);
      const hash = await computeFileSha256(picked);
      onChange({ kind: "sha256", hash });
    } catch (e) {
      setError(String(e));
    } finally {
      setComputing(false);
    }
  };

  return (
    <div className="space-y-1">
      <Button
        type="button"
        size="sm"
        variant="outline"
        disabled={computing}
        onClick={() => void handleChangeFile()}
        className="cursor-pointer"
      >
        {computing ? "계산 중..." : "압축 파일 변경"}
      </Button>
      {error && <p className="break-all text-[11px] text-red-300">계산 실패: {error}</p>}
    </div>
  );
}

// ---------------------------------------------------------------------------
// files 편집 — 화이트리스트 + 오버라이드
// ---------------------------------------------------------------------------


/** 트리 한 행 — 폴더 또는 파일. `flattenVisibleTreeRows`가 만드는 평평한 배열의 원소. */
interface TreeRow {
  key: string;
  depth: number;
  data: { kind: "folder"; folder: FileTreeFolder } | { kind: "file"; name: string; index: number };
}

/** 트리를 지금 펼쳐진 상태(`collapsed`) 기준으로 "실제로 보이는 행"만 평평한 배열로
 * 만든다 — 이 배열에 `useVirtualizer`(아래 `ListEditor`와 같은 패턴)를 적용해 화면에
 * 실제 보이는 몇십 개 행만 DOM에 그린다. 수백~수천 개 파일이 있는 레시피(예: 대용량
 * 모드팩)에서 트리 전체를 그대로 그리면, 파일 하나만 지워도 그 많은 노드를 전부
 * 재조정해야 해서 눈에 띄게 느려짐 — 실사용 중 발견. 펼침/접힘 상태를 폴더별 컴포넌트
 * 로컬 state 대신 `FileTreeView`가 경로 집합으로 들고 있는 이유도 이 평탄화 때문 —
 * 컴포넌트 트리 밖에서 "지금 뭐가 보이는가"를 알아야 배열을 만들 수 있다. */
function flattenVisibleTreeRows(folder: FileTreeFolder, collapsed: Set<string>, depth = 0): TreeRow[] {
  const sortedFolders = Array.from(folder.folders.values()).sort((a, b) => a.name.localeCompare(b.name));
  const sortedFiles = [...folder.files].sort((a, b) => a.name.localeCompare(b.name));
  const rows: TreeRow[] = [];
  for (const f of sortedFolders) {
    rows.push({ key: `folder:${f.path}`, depth, data: { kind: "folder", folder: f } });
    if (!collapsed.has(f.path)) {
      rows.push(...flattenVisibleTreeRows(f, collapsed, depth + 1));
    }
  }
  for (const file of sortedFiles) {
    rows.push({ key: `file:${file.index}`, depth, data: { kind: "file", name: file.name, index: file.index } });
  }
  return rows;
}

const TREE_ROW_HEIGHT = 22;

function FileTreeView({
  root,
  files,
  optionalGroups,
  selectedIndex,
  onSelect,
  onAddAt,
  onRemove,
  onRemoveFolder,
  folderRules,
  selectedFolderPath,
  onSelectFolder,
}: {
  root: FileTreeFolder;
  files: RecipeFile[];
  optionalGroups: OptionalGroup[];
  selectedIndex: number | null;
  onSelect: (index: number) => void;
  onAddAt: (folderPath: string) => void;
  onRemove: (index: number) => void;
  onRemoveFolder: (path: string) => void;
  folderRules: FolderRule[];
  selectedFolderPath: string | null;
  onSelectFolder: (path: string) => void;
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

  const rows = useMemo(() => flattenVisibleTreeRows(root, collapsed), [root, collapsed]);
  const isEmpty = rows.length === 0;

  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => TREE_ROW_HEIGHT,
    overscan: 12,
  });

  return (
    <div className="rounded border border-neutral-800 bg-neutral-950/40 p-2">
      {isEmpty && <p className="px-1 py-2 text-xs text-neutral-500">아직 파일이 없습니다.</p>}
      {!isEmpty && (
        <div ref={scrollRef} className="max-h-[20vh] overflow-y-auto">
          <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
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
                      selected={selectedFolderPath === data.folder.path}
                      onSelectFolder={onSelectFolder}
                      onAddAt={onAddAt}
                      onRemoveFolder={onRemoveFolder}
                    />
                  ) : (
                    <FileLeaf
                      name={data.name}
                      index={data.index}
                      file={files[data.index]}
                      optionalGroups={optionalGroups}
                      depth={row.depth}
                      selected={selectedIndex === data.index}
                      onSelect={onSelect}
                      onRemove={onRemove}
                    />
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
      <button
        type="button"
        onClick={() => onAddAt("")}
        className="mt-1 cursor-pointer px-1 text-xs text-neutral-400 hover:text-neutral-200"
      >
        + 새 파일 추가(루트)
      </button>
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
  onSelectFolder,
  onAddAt,
  onRemoveFolder,
}: {
  folder: FileTreeFolder;
  depth: number;
  expanded: boolean;
  onToggleExpanded: () => void;
  rule: FolderRule | undefined;
  selected: boolean;
  onSelectFolder: (path: string) => void;
  onAddAt: (folderPath: string) => void;
  onRemoveFolder: (path: string) => void;
}) {
  return (
    <div
      className={`group flex h-full items-center gap-1 rounded px-1 ${
        selected ? "bg-neutral-800" : "hover:bg-neutral-800/50"
      }`}
      style={{ paddingLeft: depth * 14 }}
    >
      <button
        type="button"
        onClick={onToggleExpanded}
        className="w-4 shrink-0 cursor-pointer text-neutral-500"
      >
        {expanded ? "▾" : "▸"}
      </button>
      <button
        type="button"
        onClick={() => onSelectFolder(folder.path)}
        className={`flex-1 cursor-pointer truncate text-left text-xs ${
          selected ? "text-neutral-100" : "text-neutral-300"
        }`}
      >
        📁 {folder.name}
      </button>
      {rule && (
        <span className="shrink-0 whitespace-nowrap rounded bg-neutral-100 px-1 text-[10px] font-medium text-neutral-900 group-hover:hidden">
          {folderRuleBadgeLabel(rule.mode)}
        </span>
      )}
      <button
        type="button"
        onClick={() => onAddAt(folder.path)}
        className="hidden shrink-0 cursor-pointer px-1 text-[11px] text-neutral-500 hover:text-neutral-200 group-hover:inline"
        title="여기에 새 파일 추가"
      >
        +
      </button>
      <button
        type="button"
        onClick={() => onRemoveFolder(folder.path)}
        className="hidden shrink-0 cursor-pointer px-1 text-red-300 hover:text-red-200 group-hover:inline"
        title="이 폴더와 안의 파일 전부 제거"
        aria-label="폴더 제거"
      >
        ✕
      </button>
    </div>
  );
}

function folderRuleBadgeLabel(mode: FolderRuleMode): string {
  return mode.kind === "passthrough" ? "전체 허용" : "필터링";
}

function FolderRuleEditor({
  mode,
  onChange,
  copiedRule,
  onCopy,
}: {
  mode: FolderRuleMode | null;
  onChange: (mode: FolderRuleMode | null) => void;
  /** 다른 폴더에서 복사해둔 규칙 — 있으면 "붙여넣기" 버튼이 활성화된다. */
  copiedRule: FolderRuleMode | null;
  onCopy: (mode: FolderRuleMode) => void;
}) {
  // 규칙 없음(mode === null)은 패턴 0개짜리 필터링과 동작이 완전히 같다(화이트리스트만
  // 허용) — 그래서 "기본"을 별도 선택지로 안 두고 필터링에 합친다. 패턴을 하나도 안
  // 넣으면 지금과 똑같이 동작하고, 넣으면 그만큼 허용 범위가 넓어진다.
  const kind: "filtered" | "passthrough" = mode?.kind === "passthrough" ? "passthrough" : "filtered";
  const patterns = mode?.kind === "filtered" ? mode.patterns : [];
  const disallowPatterns = mode?.kind === "filtered" ? mode.disallow_patterns : [];
  // 지금 화면에 보이는 그대로(규칙 없음이어도 필터링 빈 값과 동일하게 취급)를
  // 복사한다 — mode 를 그대로 복사하면 "규칙 없음"(null)을 복사하는 셈이 돼 붙여넣기
  // 대상이 뭘 받는지 불분명해진다.
  const effectiveMode: FolderRuleMode =
    kind === "passthrough" ? { kind: "passthrough" } : { kind: "filtered", patterns, disallow_patterns: disallowPatterns };
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <Select
          value={kind}
          onChange={(k) => {
            if (k === "passthrough") onChange({ kind: "passthrough" });
            else onChange({ kind: "filtered", patterns, disallow_patterns: disallowPatterns });
          }}
          options={[
            { value: "passthrough", label: "전체 허용" },
            { value: "filtered", label: "필터링" },
          ]}
        />
        <button
          type="button"
          onClick={() => onCopy(effectiveMode)}
          className="shrink-0 cursor-pointer text-xs text-neutral-400 hover:text-neutral-200"
        >
          규칙 복사
        </button>
        <button
          type="button"
          disabled={!copiedRule}
          onClick={() => copiedRule && onChange(copiedRule)}
          className="shrink-0 cursor-pointer text-xs text-neutral-400 hover:text-neutral-200 disabled:cursor-not-allowed disabled:opacity-40"
        >
          규칙 붙여넣기
        </button>
      </div>
      {kind === "filtered" && (
        <>
          <Field label="허용 (이 폴더 기준 상대 글롭)">
            <PatternListEditor
              patterns={patterns}
              onChange={(next) =>
                onChange({ kind: "filtered", patterns: next, disallow_patterns: disallowPatterns })
              }
            />
          </Field>
          <Field label="비허용 (허용 패턴으로 들어온 것 중 제외 — 선언된 파일은 항상 남음)">
            <PatternListEditor
              patterns={disallowPatterns}
              onChange={(next) => onChange({ kind: "filtered", patterns, disallow_patterns: next })}
              emptyLabel="제외 없음 — 허용 패턴에 걸리면 전부 남음"
            />
          </Field>
        </>
      )}
    </div>
  );
}

/** 글롭 패턴 목록 편집 — 백엔드가 `BTreeSet`(중복 없음, 정렬)으로 저장하므로 여기서도
 * 추가할 때 중복을 막고 정렬된 순서로 보여준다(순서 자체엔 의미 없음 — 매칭은
 * "하나라도 맞으면 허용"). */
function PatternListEditor({
  patterns,
  onChange,
  emptyLabel = "필터링 없음 — 선언된 파일만 허용",
}: {
  patterns: string[];
  onChange: (patterns: string[]) => void;
  emptyLabel?: string;
}) {
  const [draft, setDraft] = useState("");
  const add = () => {
    const trimmed = draft.trim();
    if (!trimmed || patterns.includes(trimmed)) return;
    onChange([...patterns, trimmed].sort());
    setDraft("");
  };
  return (
    <div className="space-y-1">
      {patterns.length === 0 && (
        <p className="text-[11px] text-neutral-500">{emptyLabel}</p>
      )}
      {patterns.map((p) => (
        <div key={p} className="flex items-center gap-1">
          <span className="min-w-0 flex-1 truncate rounded bg-neutral-950 px-1.5 py-0.5 font-mono text-[11px] text-neutral-300">
            {p}
          </span>
          <button
            type="button"
            onClick={() => onChange(patterns.filter((x) => x !== p))}
            className="shrink-0 cursor-pointer text-red-300 hover:text-red-200"
            aria-label="삭제"
          >
            ✕
          </button>
        </div>
      ))}
      <div className="flex gap-1">
        <TextInput value={draft} placeholder="예: *.sav" onChange={setDraft} />
        <Button type="button" size="sm" variant="outline" className="shrink-0 cursor-pointer" onClick={add}>
          추가
        </Button>
      </div>
    </div>
  );
}

function FileLeaf({
  name,
  index,
  file,
  optionalGroups,
  depth,
  selected,
  onSelect,
  onRemove,
}: {
  name: string;
  index: number;
  file: RecipeFile;
  optionalGroups: OptionalGroup[];
  depth: number;
  selected: boolean;
  onSelect: (index: number) => void;
  onRemove: (index: number) => void;
}) {
  const groupLabel = file.optional_group
    ? (optionalGroups.find((g) => g.id === file.optional_group)?.label ?? file.optional_group)
    : null;
  const contentLabel = file.override_content ? OVERRIDE_KIND_LABELS[file.override_content.kind] : null;
  return (
    <div
      className={`group flex h-full items-center gap-1 rounded px-1 text-xs ${
        selected
          ? "bg-neutral-800 text-neutral-100"
          : "text-neutral-400 hover:bg-neutral-800/50 hover:text-neutral-200"
      }`}
      style={{ paddingLeft: depth * 14 + 20 }}
    >
      <button
        type="button"
        onClick={() => onSelect(index)}
        className="flex-1 cursor-pointer truncate text-left"
      >
        📄 {name || "(이름 없음)"}
      </button>
      {groupLabel && (
        <span className="shrink-0 whitespace-nowrap rounded bg-neutral-100 px-1 text-[10px] font-medium text-neutral-900 group-hover:hidden">
          {groupLabel}
        </span>
      )}
      {contentLabel && (
        <span className="shrink-0 whitespace-nowrap rounded bg-neutral-100 px-1 text-[10px] font-medium text-neutral-900 group-hover:hidden">
          {contentLabel}
        </span>
      )}
      <button
        type="button"
        onClick={() => onRemove(index)}
        className="hidden shrink-0 cursor-pointer text-red-300 hover:text-red-200 group-hover:inline"
        aria-label="삭제"
      >
        ✕
      </button>
    </div>
  );
}

/** 설치 조건(`optional_group`)은 여기서 편집 안 한다 — 압축 하나가 곧 옵션 하나이고
 * (2026-08, 사용자 확인), 이 화면이 다루는 화이트리스트 파일은 그 압축이 목적지
 * 폴더를 통째로 소유하는 구조상 이미 압축 선택 여부에 자동으로 딸려간다. 압축과
 * 무관하게 파일 하나만 별도 옵션으로 묶는 흐름 자체를 없앴다. */
function RecipeFileEditor({
  file,
  onChange,
  onRemove,
}: {
  file: RecipeFile;
  onChange: (f: RecipeFile) => void;
  onRemove: () => void;
}) {
  return (
    <div className="space-y-2">
      <div className="flex justify-end">
        <RemoveButton onClick={onRemove} />
      </div>
      <Field label="경로">
        <TextInput
          value={file.path}
          placeholder="대상 루트 기준 상대경로 (예: SampleApp/option.ini)"
          onChange={(path) => onChange({ ...file, path })}
        />
      </Field>
      <Field label="내용">
        <OverrideContentFields
          content={file.override_content ?? null}
          onChange={(override_content) => onChange({ ...file, override_content })}
        />
      </Field>
    </div>
  );
}

function OverrideContentFields({
  content,
  onChange,
}: {
  content: OverrideContent | null;
  onChange: (c: OverrideContent | null) => void;
}) {
  const kind: OverrideKind = content?.kind ?? "none";
  return (
    <div className="space-y-2">
      <Field label="지정 방식">
        <Select
          value={kind}
          onChange={(k) => {
            if (k === "none") onChange(null);
            else if (k === "literal") onChange({ kind: "literal", content: { encoding: "text", content: "" } });
            else onChange({ kind: "config_patch", format: "ini", patch: {} });
          }}
          options={(Object.keys(OVERRIDE_KIND_LABELS) as OverrideKind[]).map((k) => ({
            value: k,
            label: OVERRIDE_KIND_LABELS[k],
          }))}
        />
      </Field>
      {content?.kind === "literal" && (
        <FileContentFields
          content={content.content}
          onChange={(c) => onChange({ kind: "literal", content: c })}
        />
      )}
      {content?.kind === "config_patch" && (
        <ConfigPatchFields
          format={content.format}
          patch={content.patch}
          onFormatChange={(format) => onChange({ kind: "config_patch", format, patch: content.patch })}
          onPatchChange={(patch) => onChange({ kind: "config_patch", format: content.format, patch })}
        />
      )}
    </div>
  );
}

function FileContentFields({
  content,
  onChange,
}: {
  content: FileContent;
  onChange: (c: FileContent) => void;
}) {
  return (
    <div className="space-y-2">
      <Field label="인코딩">
        <Select
          value={content.encoding}
          onChange={(encoding) =>
            onChange(
              encoding === "text" ? { encoding: "text", content: "" } : { encoding: "base64", data: "" },
            )
          }
          options={[
            { value: "text", label: "텍스트" },
            { value: "base64", label: "base64 (바이너리)" },
          ]}
        />
      </Field>
      <Field label="내용">
        {content.encoding === "text" ? (
          <textarea
            className={`${inputClass} min-h-[80px] font-mono text-xs`}
            value={content.content}
            onChange={(e) => onChange({ encoding: "text", content: e.target.value })}
          />
        ) : (
          <textarea
            className={`${inputClass} min-h-[80px] font-mono text-xs`}
            value={content.data}
            placeholder="base64 문자열"
            onChange={(e) => onChange({ encoding: "base64", data: e.target.value })}
          />
        )}
      </Field>
    </div>
  );
}

/** ini/json/toml 공통 — patch 는 항상 JSON(포맷 무관). ini/toml 은
 * `{"섹션": {"키": 값}}`, json 은 파일 최상위 구조 그 자체. 직접 손으로 JSON 을 쓰거나,
 * 이미 갖고 있는 설정 파일을 "파일에서 불러오기"로 통째로 채워 넣을 수 있다(예: 직접
 * 쓰던 option.ini) — 이 값은 레시피 자체에 실려 공유된다. */
function ConfigPatchFields({
  format,
  patch,
  onFormatChange,
  onPatchChange,
}: {
  format: ConfigFileFormat;
  patch: ConfigPatch;
  onFormatChange: (f: ConfigFileFormat) => void;
  onPatchChange: (p: ConfigPatch) => void;
}) {
  const [text, setText] = useState(() => JSON.stringify(patch, null, 2));
  const [parseError, setParseError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);

  // patch 가 외부에서 바뀌면(불러오기 성공 등) textarea 도 동기화.
  useEffect(() => {
    setText(JSON.stringify(patch, null, 2));
  }, [patch]);

  const handleTextChange = (v: string) => {
    setText(v);
    try {
      const parsed = JSON.parse(v) as ConfigPatch;
      setParseError(null);
      onPatchChange(parsed);
    } catch (e) {
      setParseError(String(e));
    }
  };

  const handleImport = async () => {
    setImportError(null);
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({ multiple: false, title: "불러올 설정 파일 선택" });
      if (!picked || typeof picked !== "string") return;
      setImporting(true);
      const imported = await readConfigFileAsPatch(picked, format);
      onPatchChange(imported);
    } catch (e) {
      setImportError(String(e));
    } finally {
      setImporting(false);
    }
  };

  return (
    <div className="space-y-2">
      <Field label="설정 파일 형식">
        <Select
          value={format}
          onChange={(f) => onFormatChange(f as ConfigFileFormat)}
          options={[
            { value: "ini", label: "ini" },
            { value: "json", label: "json" },
            { value: "toml", label: "toml" },
          ]}
        />
      </Field>
      <Button
        type="button"
        size="sm"
        variant="outline"
        disabled={importing}
        onClick={() => void handleImport()}
        className="cursor-pointer"
      >
        {importing ? "불러오는 중..." : "파일에서 불러오기"}
      </Button>
      {importError && (
        <p className="break-all text-[11px] text-red-300">불러오기 실패: {importError}</p>
      )}
      <Field label="변경할 값">
        <p className="text-[11px] text-neutral-500">
          패치 값(JSON) — ini/toml은 <code>{"{ \"섹션\": { \"키\": \"값\" } }"}</code> 형태
        </p>
        <textarea
          className={`${inputClass} min-h-[100px] font-mono text-xs`}
          value={text}
          onChange={(e) => handleTextChange(e.target.value)}
        />
        {parseError && <p className="text-[11px] text-red-300">JSON 형식 오류: {parseError}</p>}
      </Field>
    </div>
  );
}

// ---------------------------------------------------------------------------
// launch 필드
// ---------------------------------------------------------------------------

function LaunchActionFields({
  launch,
  files,
  folderRules,
  onChange,
}: {
  launch: LaunchAction;
  files: RecipeFile[];
  folderRules: FolderRule[];
  onChange: (l: LaunchAction) => void;
}) {
  // 등록된 third-party app id 목록(`resources/third_party_apps.json`) — 자유 텍스트
  // 대신 여기서 고르게 한다. 마운트 시 1회만 조회(자주 안 바뀌는 번들 자산).
  const [thirdPartyAppIds, setThirdPartyAppIds] = useState<string[] | null>(null);
  useEffect(() => {
    let cancelled = false;
    void listThirdPartyAppIds().then((ids) => {
      if (!cancelled) setThirdPartyAppIds(ids);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  // 목록이 로드됐는데 아직 app_id 가 안 정해져 있으면(신규 레시피 작성 시 기본값)
  // 목록의 첫 항목으로 채운다 — 특정 app_id 를 코드에 미리 박아두지 않기 위함(등록된
  // 앱이 둘 이상이 되면 "누가 기본"인지는 목록 순서가 정할 뿐, 코드가 정하지 않는다).
  useEffect(() => {
    if (
      launch.kind === "third_party_app_launch" &&
      launch.app_id === "" &&
      thirdPartyAppIds &&
      thirdPartyAppIds.length > 0
    ) {
      onChange({ ...launch, app_id: thirdPartyAppIds[0] });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [launch, thirdPartyAppIds]);

  if (launch.kind === "spawn_process") {
    return (
      <>
        <Field label="실행 파일 (앱 루트 폴더 기준 상대경로)">
          <div className="flex items-center gap-2">
            <div className="min-w-0 flex-1">
              <TextInput
                value={launch.entry_point}
                placeholder="예: SampleApp/Launcher.exe"
                readOnly
                onChange={(entry_point) => onChange({ ...launch, entry_point })}
              />
            </div>
            <DestinationPathPicker
              files={files}
              mode="file"
              onPick={(entry_point) => onChange({ ...launch, entry_point })}
              extraFolderPaths={folderRules.map((r) => r.path)}
            />
          </div>
        </Field>
        <Field label="실행 인자 (쉼표로 구분, 선택)">
          <TextInput
            value={launch.entry_args.join(", ")}
            onChange={(v) =>
              onChange({
                ...launch,
                entry_args: v
                  .split(",")
                  .map((s) => s.trim())
                  .filter((s) => s.length > 0),
              })
            }
          />
        </Field>
      </>
    );
  }
  return (
    <>
      <Field label="대상 서드파티 앱">
        {thirdPartyAppIds === null ? (
          <TextInput value={launch.app_id} disabled onChange={() => {}} placeholder="목록 불러오는 중..." />
        ) : (
          <Select
            value={launch.app_id}
            // 기존 레시피가 목록에 없는 id(예: 폐기된 앱)를 참조 중이면 옵션에서
            // 조용히 사라지지 않도록 현재 값도 항상 포함.
            options={Array.from(new Set([...thirdPartyAppIds, launch.app_id])).map((id) => ({
              value: id,
              label: id,
            }))}
            onChange={(app_id) => onChange({ ...launch, app_id })}
          />
        )}
      </Field>
    </>
  );
}

// ---------------------------------------------------------------------------
// 공용 입력 프리미티브 — `Field`/`TextInput`/`Select`/`RemoveButton`/`inputClass`는
// `@/components/ui/form-fields`(두 번째 사용처인 `ThirdPartyAppEditDialog`와 공유).
// ---------------------------------------------------------------------------

/** `url`을 실제 `<img>`로 로드해봐서 실패하는지 추적 — 빈 문자열은 "아직 안 적음"이라
 * 실패로 취급하지 않는다. */
function useImageLoadFailed(url: string): boolean {
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!url) {
      setFailed(false);
      return;
    }
    let cancelled = false;
    setFailed(false);
    const img = new Image();
    img.onload = () => {
      if (!cancelled) setFailed(false);
    };
    img.onerror = () => {
      if (!cancelled) setFailed(true);
    };
    img.src = url;
    return () => {
      cancelled = true;
    };
  }, [url]);
  return failed;
}

/** `Field`의 `action` 슬롯에 넣어 라벨 우측에 붙이는 경고 아이콘 — `url`을 이미지로
 * 못 불러오면(깨진 링크, CORS 등) 표시하고, 그 외엔 아무것도 렌더링하지 않는다. */
function ImageUrlWarning({ url }: { url: string }) {
  const failed = useImageLoadFailed(url);
  if (!failed) return null;
  return (
    <span className="text-amber-400" title="이미지를 불러오지 못했습니다" aria-label="이미지 로드 실패">
      ⚠
    </span>
  );
}

/** row 높이가 고정이 아니다(오버라이드 내용을 펼치면 textarea가 붙어 커짐) — 그래서
 * `measureElement`로 실측해 보정하는 가상 스크롤을 쓴다. 스크롤 컨테이너에 `max-h`를
 * 줘야 windowing이 의미가 있는데(무제한 높이면 전부 "화면 안"이라 windowing 자체가
 * 안 됨), 항목이 적어 그 높이를 안 넘기면 스크롤바가 아예 안 생겨 지금과 시각적으로
 * 동일 — 그래서 항목 수 기준 분기를 따로 두지 않는다. */
function ListEditor<T>({
  items,
  onChange,
  newItem,
  renderItem,
  addLabel,
}: {
  items: T[];
  onChange: (items: T[]) => void;
  newItem: () => T;
  renderItem: (item: T, update: (item: T) => void, remove: () => void, index: number) => React.ReactNode;
  addLabel: string;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 120,
    overscan: 8,
  });

  return (
    <div className="space-y-2">
      <div ref={scrollRef} className="max-h-[45vh] overflow-y-auto">
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((virtualRow) => {
            const i = virtualRow.index;
            const item = items[i];
            return (
              <div
                key={i}
                data-index={i}
                ref={virtualizer.measureElement}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  transform: `translateY(${virtualRow.start}px)`,
                }}
                className="pb-2"
              >
                <div className="rounded border border-neutral-800 bg-neutral-950/40 p-2">
                  {renderItem(
                    item,
                    (next) => onChange(items.map((it, j) => (j === i ? next : it))),
                    () => onChange(items.filter((_, j) => j !== i)),
                    i,
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
      <button
        type="button"
        onClick={() => onChange([...items, newItem()])}
        className="cursor-pointer text-xs text-neutral-400 hover:text-neutral-200"
      >
        {addLabel}
      </button>
    </div>
  );
}

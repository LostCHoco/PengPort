// 앱 라이브러리 (메인 페이지) — 0.2.0 flat 모델. 옛 PspLibrary(인스턴스→카탈로그→매니페스트
// 3단 체인) 대체.
//
// 흐름:
// 1. `library_list` 로 로컬 레시피 전체를 flat 하게 불러와 그리드로 렌더링(그루핑 없음).
// 2. 설치/실행 두 버튼 — "설치" 버튼은 처음 설치든 이미 설치된 레시피의 변경분
//    반영("업데이트")이든 같은 커맨드(`library_install`, 지금 레시피와 실제 설치
//    상태가 다른 스텝만 적용). 실행(`library_launch`)은 설치를 자동으로 하지 않는다
//    — 사용자가 각각 명시적으로 누른다.
// 3. 어느 쪽이든 `third_party_app_missing` outcome 이면 `ThirdPartyInstallDialog` 로
//    Prism 등을 설치받고, 원래 눌렀던 동작(설치 또는 실행)을 그대로 재시도.
// 4. 링크 임포트는 `App.tsx` 의 딥링크 핸들러가 `ImportDialog` 를 띄우고, 성공하면
//    `reloadKey` 를 이 페이지에 넘겨 refetch — Library 페이지 자체는 딥링크를 모른다.

import { useCallback, useEffect, useRef, useState } from "react";
import { useOutletContext } from "react-router";
import { AppCard } from "@/components/AppCard";
import { ArchiveConflictDialog, type ArchiveConflictResolved } from "@/components/ArchiveConflictDialog";
import { DeleteInstalledDataDialog } from "@/components/DeleteInstalledDataDialog";
import { OptionalGroupsDialog } from "@/components/OptionalGroupsDialog";
import { OverrideConflictDialog } from "@/components/OverrideConflictDialog";
import { LocalRootOverrideDialog } from "@/components/LocalRootOverrideDialog";
import { RecipeEditDialog } from "@/components/RecipeEditDialog";
import { ThirdPartyInstallDialog } from "@/components/ThirdPartyInstallDialog";
import { Button } from "@/components/ui/button";
import { Portal } from "@/components/ui/portal";
import { useConfirmDialog } from "@/components/ui/confirm-dialog";
import {
  libraryCancelInstall,
  libraryDeleteInstalledData,
  libraryExportFile,
  libraryGet,
  libraryInstall,
  libraryLaunch,
  libraryList,
  libraryOpenFolder,
  libraryRemove,
  libraryReorder,
  libraryResolveArchiveConflicts,
  libraryResolveOverrideConflicts,
  librarySetSelectedOptionalGroups,
  libraryUpsert,
} from "@/lib/library";
import type {
  ArchiveConflictGroup,
  InstallOutcome,
  LaunchOutcome,
  OverrideConflict,
  OverrideConflictResolution,
  Recipe,
  RecipeSummary,
} from "@/lib/library";

interface ToastState {
  kind: "info" | "error";
  message: string;
}

/** third-party app(예: Prism)이 없어서 멈춘 동작 — 설치받은 뒤 원래 동작을 재시도해야
 * 해서 "무엇을 하려던 참이었는지" 같이 기억해둔다. */
interface PendingThirdPartyInstall {
  recipe: RecipeSummary;
  retry: "install" | "launch";
}

export default function Library() {
  // App.tsx 가 <Outlet context={reloadKey} /> 로 넘김 — 임포트 성공 시 bump.
  const reloadKey = useOutletContext<number>();
  const { confirmAsync, dialog: confirmDialog } = useConfirmDialog();
  const [recipes, setRecipes] = useState<RecipeSummary[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  // 카드 드래그 재정렬 — 끄는 중인 카드 id. 놓는 순간 실제 배열 순서를 바꾼다.
  const [draggedId, setDraggedId] = useState<string | null>(null);
  // 지금 가장 가까운(놓으면 자리를 맞바꿀) 카드 id — 은은한 미리보기 강조용.
  const [dragOverId, setDragOverId] = useState<string | null>(null);
  // 카드별 DOM 엘리먼트 — 카드 사이 그리드 gap 위에서 놓아도 "가장 가까운 카드"를
  // 좌표로 계산하기 위함(개별 카드에 걸린 dragover/drop 이벤트만으론 gap 픽셀 위에서
  // 놓았을 때 아무 카드에도 안 걸려서 무시되는 문제가 있었다).
  const cardRefs = useRef<Map<string, HTMLLIElement>>(new Map());

  const findNearestRecipeId = useCallback((clientX: number, clientY: number): string | null => {
    let nearestId: string | null = null;
    let nearestDistSq = Infinity;
    for (const [id, el] of cardRefs.current) {
      const rect = el.getBoundingClientRect();
      const dx = clientX - (rect.left + rect.width / 2);
      const dy = clientY - (rect.top + rect.height / 2);
      const distSq = dx * dx + dy * dy;
      if (distSq < nearestDistSq) {
        nearestDistSq = distSq;
        nearestId = id;
      }
    }
    return nearestId;
  }, []);
  const [installingId, setInstallingId] = useState<string | null>(null);
  const [launchingId, setLaunchingId] = useState<string | null>(null);
  // 설치/실행 시도가 끝날 때마다 bump — AppCard 가 이걸 보고 설치 상태
  // 뱃지를 다시 조회한다(직접 폴링하지 않고, 바뀔 만한 시점에만).
  const [statusVersion, setStatusVersion] = useState(0);
  const [pendingInstall, setPendingInstall] = useState<PendingThirdPartyInstall | null>(null);
  // 선택적 그룹(부분 설치) 확인 다이얼로그 — `handleInstall`이 그룹 있는 레시피면
  // "설치" 버튼을 누를 때마다 매번 연다.
  const [pendingOptionalGroups, setPendingOptionalGroups] = useState<RecipeSummary | null>(null);
  // override 파일 드리프트 충돌 다이얼로그 — `libraryInstall`이
  // `has_override_conflicts`를 반환하면 열림(`handleInstallOutcome` 참고).
  const [pendingOverrideConflicts, setPendingOverrideConflicts] = useState<{
    recipe: RecipeSummary;
    conflicts: OverrideConflict[];
  } | null>(null);
  // 압축 해제 파일명 충돌 다이얼로그 — `libraryInstall`이 `has_archive_conflicts`를
  // 반환하면 열림(`handleInstallOutcome` 참고).
  const [pendingArchiveConflicts, setPendingArchiveConflicts] = useState<{
    recipe: RecipeSummary;
    archives: ArchiveConflictGroup[];
  } | null>(null);
  // "로컬 폴더 연결" 다이얼로그 — 카드 메뉴에서 열림.
  const [pendingLocalRootOverride, setPendingLocalRootOverride] = useState<RecipeSummary | null>(null);
  // 설치된 데이터 삭제 확인 — 선택적 그룹이 있는 레시피만 이 다이얼로그를 거친다
  // (전체/부분 삭제 선택). 그룹이 없으면 `handleDelete`가 바로 native confirm 처리.
  const [pendingDelete, setPendingDelete] = useState<RecipeSummary | null>(null);
  // 편집 다이얼로그에 넘길 전체 `Recipe`(콘텐츠 포함) — 그리드가 들고 있는
  // `RecipeSummary`엔 콘텐츠가 없어서, 편집을 열 때만 `libraryGet`으로 따로 받는다
  // (`handleEditRequest` 참고).
  const [editingRecipe, setEditingRecipe] = useState<Recipe | null>(null);
  const [creatingNew, setCreatingNew] = useState(false);
  const [toast, setToast] = useState<ToastState | null>(null);
  // 여러 앱 선택 모드 — 켜지면 카드마다 체크박스가 나타나고, 하나 이상 선택되면
  // 내보내기/삭제/라이브러리에서 제거를 한꺼번에 적용하는 도구모음이 뜬다.
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  // "에러 복사" 버튼을 누른 직후 잠깐 "복사됨"으로 바꿔 보여주는 용도 — 토스트 자체가
  // 바뀌거나 사라지면(새 토스트, 닫기) 자연히 초기화된다.
  const [copiedToast, setCopiedToast] = useState(false);
  const dialogOpen = editingRecipe !== null || creatingNew;
  const closeDialog = useCallback(() => {
    setEditingRecipe(null);
    setCreatingNew(false);
  }, []);

  // 토스트가 바뀌거나 사라질 때마다 "복사됨" 표시를 초기화 — 다음 토스트(또는 재조회)에
  // 이전 복사 상태가 그대로 남아 보이지 않게.
  useEffect(() => {
    setCopiedToast(false);
  }, [toast]);

  const handleCopyToastMessage = async (message: string) => {
    try {
      const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
      await writeText(message);
      setCopiedToast(true);
    } catch (e) {
      console.warn("[Library] 에러 복사 실패", e);
    }
  };

  const refresh = useCallback(async () => {
    try {
      const list = await libraryList();
      setRecipes(list);
      setLoadError(null);
    } catch (e) {
      setLoadError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh, reloadKey]);

  const handleInstallOutcome = useCallback((recipe: RecipeSummary, outcome: InstallOutcome) => {
    switch (outcome.kind) {
      case "completed":
        setToast({
          kind: "info",
          message:
            outcome.updated === 0
              ? `${recipe.name}: 이미 최신 상태입니다`
              : `${recipe.name}: ${outcome.updated}/${outcome.total}개 항목 설치/갱신됨`,
        });
        return;
      case "using_local_override":
        setToast({
          kind: "info",
          message: `${recipe.name}: 로컬 폴더를 직접 사용 중이라 설치 단계가 없습니다`,
        });
        return;
      case "third_party_app_missing":
        setPendingInstall({ recipe, retry: "install" });
        return;
      case "needs_optional_group_selection":
        setPendingOptionalGroups(recipe);
        return;
      case "cancelled":
        // 에러가 아니라 사용자가 직접 취소를 눌러서 생긴 정상적인 결과 — 빨간 에러
        // 토스트 대신 조용한 안내만.
        setToast({ kind: "info", message: `${recipe.name}: 설치를 취소했습니다` });
        return;
      case "has_override_conflicts":
        setPendingOverrideConflicts({ recipe, conflicts: outcome.conflicts });
        return;
      case "has_archive_conflicts":
        setPendingArchiveConflicts({ recipe, archives: outcome.archives });
        return;
    }
  }, []);

  const handleCancelInstall = useCallback((recipe: RecipeSummary) => {
    void libraryCancelInstall(recipe.id);
  }, []);

  const doInstall = useCallback(
    async (recipe: RecipeSummary) => {
      setInstallingId(recipe.id);
      try {
        const outcome = await libraryInstall(recipe.id);
        handleInstallOutcome(recipe, outcome);
      } catch (e) {
        setToast({ kind: "error", message: String(e) });
      } finally {
        setInstallingId(null);
        setStatusVersion((v) => v + 1);
      }
    },
    [handleInstallOutcome],
  );

  /** "설치" 버튼의 실제 클릭 핸들러 — 선택적 그룹이 있으면(부분 설치 가능)
   * 매번 먼저 구성 요소 선택 다이얼로그를 띄우고, 확정되면 `doInstall`이 실행된다
   * (`handleOptionalGroupsConfirmed` 참고). 그룹이 아예 없는 레시피만 다이얼로그 없이
   * 바로 설치 — "선택 필요" 뱃지로 첫 설치 때만 알려주던 옛 방식 대신, 설치할 때마다
   * 지금 선택을 확인/조정할 기회를 준다. */
  const handleInstall = useCallback(
    async (recipe: RecipeSummary) => {
      if (recipe.optional_groups.length > 0) {
        setPendingOptionalGroups(recipe);
        return;
      }
      await doInstall(recipe);
    },
    [doInstall],
  );

  const handleLaunchOutcome = useCallback((recipe: RecipeSummary, outcome: LaunchOutcome) => {
    switch (outcome.kind) {
      case "launched":
        setToast({ kind: "info", message: `${recipe.name} 실행 시작` });
        return;
      case "third_party_app_missing":
        setPendingInstall({ recipe, retry: "launch" });
        return;
    }
  }, []);

  const handleLaunch = useCallback(
    async (recipe: RecipeSummary) => {
      setLaunchingId(recipe.id);
      try {
        const outcome = await libraryLaunch(recipe.id);
        handleLaunchOutcome(recipe, outcome);
      } catch (e) {
        setToast({ kind: "error", message: String(e) });
      } finally {
        setLaunchingId(null);
        setStatusVersion((v) => v + 1);
      }
    },
    [handleLaunchOutcome],
  );

  const handleThirdPartyInstalled = useCallback(async () => {
    const pending = pendingInstall;
    setPendingInstall(null);
    if (!pending) return;
    if (pending.retry === "install") {
      // 그룹 선택은 최초 클릭(`handleInstall`)에서 이미 끝났으므로 여기선 다이얼로그를
      // 다시 띄우지 않고 바로 재시도.
      await doInstall(pending.recipe);
    } else {
      await handleLaunch(pending.recipe);
    }
  }, [pendingInstall, doInstall, handleLaunch]);

  /** 선택 확정 — 설치 버튼 클릭으로 열렸든(`handleInstall`) 메뉴로 직접 열었든, 확정하면
   * 바로 `doInstall`을 실행해서 켠 그룹은 복원되고 끈 그룹은 지워지게 한다. `handleInstall`
   * 이 아니라 `doInstall`을 직접 불러야 한다 — 안 그러면 그룹이 있는 레시피라 다이얼로그가
   * 곧바로 다시 열리는 무한 루프가 된다. */
  const handleOptionalGroupsConfirmed = useCallback(
    async (groups: string[]) => {
      const recipe = pendingOptionalGroups;
      setPendingOptionalGroups(null);
      if (!recipe) return;
      try {
        await librarySetSelectedOptionalGroups(recipe.id, groups);
      } catch (e) {
        setToast({ kind: "error", message: String(e) });
        return;
      }
      await doInstall(recipe);
    },
    [pendingOptionalGroups, doInstall],
  );

  /** override 충돌 확정 — 파일별 선택을 반영한 뒤 바로 재설치를 재시도한다(선택적
   * 그룹 확정과 같은 "해결 후 재시도" 패턴). */
  const handleOverrideConflictsResolved = useCallback(
    async (resolutions: OverrideConflictResolution[]) => {
      const pending = pendingOverrideConflicts;
      setPendingOverrideConflicts(null);
      if (!pending) return;
      try {
        await libraryResolveOverrideConflicts(pending.recipe.id, resolutions);
      } catch (e) {
        setToast({ kind: "error", message: String(e) });
        return;
      }
      await doInstall(pending.recipe);
    },
    [pendingOverrideConflicts, doInstall],
  );

  /** 압축 충돌 확정 — 그룹(압축)마다 해결 커맨드를 따로 호출한 뒤(해결 커맨드가
   * 압축 하나씩만 받음) 바로 재설치를 재시도한다. 보존해둔 다운로드를 재사용하므로
   * 재다운로드 없이 이어진다. */
  const handleArchiveConflictsResolved = useCallback(
    async (resolved: ArchiveConflictResolved[]) => {
      const pending = pendingArchiveConflicts;
      setPendingArchiveConflicts(null);
      if (!pending) return;
      try {
        for (const group of resolved) {
          await libraryResolveArchiveConflicts(pending.recipe.id, group.archiveHash, group.resolutions);
        }
      } catch (e) {
        setToast({ kind: "error", message: String(e) });
        return;
      }
      await doInstall(pending.recipe);
    },
    [pendingArchiveConflicts, doInstall],
  );

  /** 설치된 데이터를 지우되, "로컬 경로 오버라이드라 자동 삭제 안 함"(백엔드가
   * 사용자 지정 폴더를 보호하려고 던지는 에러)은 실패로 치지 않고 계속 진행한다 —
   * 그 폴더는 원래도 PengPort가 안 건드리는 게 맞아서 라이브러리 제거 자체를 막을
   * 이유가 없다. 그 외 에러(권한 등 실제 실패)만 진짜 실패로 보고 토스트 후 false —
   * 호출자는 그 항목의 라이브러리 제거를 진행하지 않는다(재시도 가능하게 남겨둠). */
  const deleteInstalledDataTolerant = useCallback(async (recipe: RecipeSummary): Promise<boolean> => {
    try {
      await libraryDeleteInstalledData(recipe.id);
      return true;
    } catch (e) {
      const message = String(e);
      if (message.includes("로컬 경로 오버라이드")) return true;
      setToast({ kind: "error", message: `${recipe.name}: ${message}` });
      return false;
    }
  }, []);

  /** "라이브러리에서 제거" — Steam 라이브러리 "제거"처럼 설치된 데이터까지 함께
   * 지우는 완전 삭제. 목록에서만 빼고 데이터는 남기는 옛 동작은 사용자 판단으로
   * 폐기(고아 데이터가 남아 정리할 방법이 사라지는 문제가 있었음) — 그 대신 확인
   * 창(`AppCard.tsx`의 `CardMenu.handleRemove`)에서 되돌릴 수 없음을 명시. */
  const handleRemove = useCallback(
    async (recipe: RecipeSummary) => {
      const ok = await deleteInstalledDataTolerant(recipe);
      if (!ok) return;
      await libraryRemove(recipe.id);
      setToast({ kind: "info", message: `${recipe.name} 삭제됨` });
      await refresh();
    },
    [refresh, deleteInstalledDataTolerant],
  );

  const runDelete = useCallback(async (recipe: RecipeSummary, groups?: string[]) => {
    try {
      await libraryDeleteInstalledData(recipe.id, groups);
      const groupLabels = groups
        ?.map((id) => recipe.optional_groups.find((g) => g.id === id)?.label ?? id)
        .join(", ");
      setToast({
        kind: "info",
        message: groupLabels
          ? `${recipe.name}: ${groupLabels} 삭제됨`
          : `${recipe.name} 설치된 파일 삭제됨`,
      });
    } catch (e) {
      setToast({ kind: "error", message: String(e) });
    } finally {
      setStatusVersion((v) => v + 1); // 삭제 후 뱃지 갱신되도록.
    }
  }, []);

  /** "삭제" 메뉴 클릭 — 선택적 그룹이 있으면 전용 다이얼로그(전체/부분 삭제 선택),
   * 없으면 간단한 confirm 하나로 바로 전체 삭제. */
  const handleDelete = useCallback(
    async (recipe: RecipeSummary) => {
      if (recipe.optional_groups.length > 0) {
        setPendingDelete(recipe);
        return;
      }
      const ok = await confirmAsync(
        `${recipe.name} 의 설치된 파일을 전부 지울까요?\n\n` +
          `게임 세이브 등 실제 데이터가 사라지며 되돌릴 수 없습니다. ` +
          `라이브러리 목록에는 그대로 남아 나중에 다시 [설치]할 수 있습니다.`,
        "warning",
      );
      if (ok) void runDelete(recipe);
    },
    [runDelete, confirmAsync],
  );

  const handleDeleteAllConfirmed = useCallback(() => {
    const recipe = pendingDelete;
    setPendingDelete(null);
    if (recipe) void runDelete(recipe);
  }, [pendingDelete, runDelete]);

  /** "재설치" 메뉴 클릭 — 설치된 파일을 전부 지우고(`runDelete`, 그룹 지정 없이 전체)
   * 곧바로 다시 설치(`doInstall`)한다. 선택적 그룹 선택 상태는 삭제로 안 지워지므로
   * (`librarySetSelectedOptionalGroups`와 별개 저장소), 재설치는 기존 선택 그대로
   * 복원된다 — 다이얼로그 다시 안 띄움. */
  const handleReinstall = useCallback(
    (recipe: RecipeSummary) => {
      void (async () => {
        const ok = await confirmAsync(
          `${recipe.name} 을(를) 재설치할까요?\n\n` +
            `설치된 파일을 전부 지우고 처음부터 다시 설치합니다. ` +
            `게임 세이브 등 실제 데이터가 사라지며 되돌릴 수 없습니다.`,
          "warning",
        );
        if (!ok) return;
        await runDelete(recipe);
        await doInstall(recipe);
      })();
    },
    [runDelete, doInstall, confirmAsync],
  );

  const handleDeleteGroupsConfirmed = useCallback(
    (groups: string[]) => {
      const recipe = pendingDelete;
      setPendingDelete(null);
      if (recipe) void runDelete(recipe, groups);
    },
    [pendingDelete, runDelete],
  );

  const handleOpenFolder = useCallback(async (recipe: RecipeSummary) => {
    try {
      await libraryOpenFolder(recipe.id);
    } catch (e) {
      setToast({ kind: "error", message: String(e) });
    }
  }, []);

  /** 딥링크(`.pengz` 파일)로 내보내기 — OS 커맨드라인 길이 한도를 피하는 링크의
   * 파일 버전(자세한 배경은 `commands/file_import.rs` 모듈 설명). 저장 위치는 OS
   * 저장 다이얼로그로 받는다. */
  const handleExport = useCallback(async (recipe: RecipeSummary) => {
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({
        title: `${recipe.name} 내보내기`,
        defaultPath: `${recipe.name}.pengz`,
        filters: [{ name: "PengPort 레시피", extensions: ["pengz"] }],
      });
      if (!path) return;
      await libraryExportFile([recipe.id], path);
      setToast({ kind: "info", message: `${recipe.name} 내보내기 완료` });
    } catch (e) {
      setToast({ kind: "error", message: String(e) });
    }
  }, []);

  const toggleSelectionMode = useCallback(() => {
    setSelectionMode((v) => !v);
    setSelectedIds(new Set());
  }, []);

  const toggleSelected = useCallback((id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  /** 선택 도구모음의 "내보내기" — 선택된 항목 전부를 레시피 하나(선택 1개) 또는
   * 번들(2개 이상)로 같은 `.pengz` 파일에 담는다. `handleExport`(카드별 단건
   * 내보내기)와 저장 다이얼로그+`libraryExportFile` 호출은 같지만, 대상이 여럿일 수
   * 있어 별도 핸들러로 둔다. */
  const handleBulkExport = useCallback(async () => {
    if (!recipes) return;
    const targets = recipes.filter((r) => selectedIds.has(r.id));
    if (targets.length === 0) return;
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({
        title: `${targets.length}개 앱 내보내기`,
        defaultPath: targets.length === 1 ? `${targets[0].name}.pengz` : "그룹.pengz",
        filters: [{ name: "PengPort 레시피", extensions: ["pengz"] }],
      });
      if (!path) return;
      await libraryExportFile(targets.map((r) => r.id), path);
      setToast({ kind: "info", message: `${targets.length}개 앱 내보내기 완료` });
      toggleSelectionMode();
    } catch (e) {
      setToast({ kind: "error", message: String(e) });
    }
  }, [recipes, selectedIds, toggleSelectionMode]);

  /** 선택 도구모음의 "삭제" — 선택된 항목들의 설치된 파일을 전부 지운다(카드별
   * 메뉴의 "삭제"와 달리, 선택적 그룹 부분삭제 다이얼로그는 여럿을 한 번에 다루기
   * 어려워 생략하고 항상 전체 삭제). 확인은 대상 이름을 다 나열한 confirm 하나로 —
   * 항목별 반복 확인 없음(1회성 confirm 원칙을 여기도 적용). */
  const handleBulkDeleteInstalled = useCallback(async () => {
    if (!recipes) return;
    const targets = recipes.filter((r) => selectedIds.has(r.id));
    if (targets.length === 0) return;
    const ok = await confirmAsync(
      `선택한 ${targets.length}개 앱의 설치된 파일을 전부 지울까요?\n\n` +
        `${targets.map((r) => r.name).join(", ")}\n\n` +
        `게임 세이브 등 실제 데이터가 사라지며 되돌릴 수 없습니다. ` +
        `라이브러리 목록에는 그대로 남아 나중에 다시 [설치]할 수 있습니다.`,
      "warning",
    );
    if (!ok) return;
    for (const recipe of targets) {
      await runDelete(recipe);
    }
    toggleSelectionMode();
  }, [recipes, selectedIds, runDelete, toggleSelectionMode, confirmAsync]);

  /** 선택 도구모음의 "라이브러리에서 제거" — 카드별 메뉴와 같은 완전 삭제(설치된
   * 파일 + 라이브러리 항목 둘 다) 동작을 선택된 항목 전부에 적용. */
  const handleBulkRemove = useCallback(async () => {
    if (!recipes) return;
    const targets = recipes.filter((r) => selectedIds.has(r.id));
    if (targets.length === 0) return;
    const ok = await confirmAsync(
      `선택한 ${targets.length}개 앱을 완전히 제거할까요?\n\n` +
        `${targets.map((r) => r.name).join(", ")}\n\n` +
        `설치된 파일들이 전부 지워지며 라이브러리 목록에서도 함께 사라집니다.`,
      "warning",
    );
    if (!ok) return;
    for (const recipe of targets) {
      const dataOk = await deleteInstalledDataTolerant(recipe);
      if (!dataOk) continue; // 실패한 항목은 라이브러리에 남겨서 재시도 가능하게, 나머지는 계속 진행.
      await libraryRemove(recipe.id);
    }
    setToast({ kind: "info", message: `${targets.length}개 앱 제거됨` });
    toggleSelectionMode();
    await refresh();
  }, [recipes, selectedIds, deleteInstalledDataTolerant, toggleSelectionMode, refresh, confirmAsync]);

  /** "앱 편집" 메뉴 클릭 — 그리드가 들고 있는 `RecipeSummary`엔 콘텐츠(archives/files의
   * override_content)가 없으므로, 편집 다이얼로그를 열 때만 `libraryGet`으로 그
   * 레시피 하나의 전체 `Recipe`를 따로 받는다. */
  const handleEditRequest = useCallback(async (id: string) => {
    try {
      const full = await libraryGet(id);
      if (!full) {
        setToast({ kind: "error", message: "레시피를 찾을 수 없습니다 — 목록을 새로고침해주세요" });
        return;
      }
      setEditingRecipe(full);
    } catch (e) {
      setToast({ kind: "error", message: String(e) });
    }
  }, []);

  const handleSaveEdit = useCallback(
    async (recipe: Recipe) => {
      await libraryUpsert(recipe);
      closeDialog();
      setToast({ kind: "info", message: `${recipe.name} 저장됨` });
      await refresh();
    },
    [refresh, closeDialog],
  );

  /** 카드를 놓은 지점에서 가장 가까운 카드(`findNearestRecipeId`)와 위치만 서로
   * 맞바꾼다(나머지는 그대로) — 카드 위든 카드 사이 gap 이든 어디에 놓아도 동작.
   * 화면을 먼저 낙관적으로 갱신하고 저장은 뒤에서 — 실패하면 서버 상태로 되돌린다. */
  const handleReorderCards = useCallback(
    (fromId: string, toId: string) => {
      if (fromId === toId) return;
      setRecipes((prev) => {
        if (!prev) return prev;
        const fromIndex = prev.findIndex((r) => r.id === fromId);
        const toIndex = prev.findIndex((r) => r.id === toId);
        if (fromIndex === -1 || toIndex === -1) return prev;
        const next = [...prev];
        [next[fromIndex], next[toIndex]] = [next[toIndex], next[fromIndex]];
        void libraryReorder(next.map((r) => r.id)).catch((e) => {
          setToast({ kind: "error", message: String(e) });
          void refresh();
        });
        return next;
      });
    },
    [refresh],
  );

  return (
    <div className="p-8">
      <header className="mb-6 flex items-center justify-between">
        <h2 className="text-2xl font-semibold">라이브러리</h2>
        <div className="flex gap-2">
          <Button
            size="sm"
            variant="outline"
            onClick={toggleSelectionMode}
            className="cursor-pointer"
          >
            {selectionMode ? "선택 취소" : "선택"}
          </Button>
          <Button size="sm" onClick={() => setCreatingNew(true)} className="cursor-pointer">
            + 새 앱 추가
          </Button>
        </div>
      </header>

      {selectionMode && selectedIds.size > 0 && (
        // 일반 문서 흐름에 넣으면(위쪽에 `mb-4` 블록으로) 나타날 때 아래 카드 그리드
        // 전체가 밀려 내려가 버려서(선택할 때마다 카드 위치가 흔들림), `fixed`로 화면에
        // 떠 있게 해 레이아웃에 전혀 영향을 안 주게 한다 — 토스트(우하단)와 안 겹치게
        // 화면 하단 중앙에 배치.
        <Portal>
          <div className="fixed bottom-6 left-1/2 z-40 flex -translate-x-1/2 items-center gap-3 rounded-lg border border-blue-900/50 bg-neutral-900 px-4 py-2.5 text-sm shadow-2xl">
            <span className="text-neutral-300">{selectedIds.size}개 선택됨</span>
            <div className="flex gap-2">
              <Button
                size="sm"
                variant="outline"
                onClick={() => void handleBulkExport()}
                className="cursor-pointer"
              >
                내보내기
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => void handleBulkDeleteInstalled()}
                className="cursor-pointer"
              >
                삭제
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() => void handleBulkRemove()}
                className="cursor-pointer text-red-300 hover:text-red-200"
              >
                라이브러리에서 제거
              </Button>
            </div>
          </div>
        </Portal>
      )}

      {loadError && (
        <div className="mb-4 space-y-2 rounded-md border border-red-900/50 bg-red-900/20 p-4 text-sm text-red-200">
          <p className="font-medium">라이브러리를 불러오지 못했습니다</p>
          <p className="text-xs text-red-300/80 break-all">{loadError}</p>
          <Button size="sm" variant="outline" onClick={() => void refresh()}>
            다시 시도
          </Button>
        </div>
      )}

      {recipes === null && !loadError && (
        <p className="text-sm text-neutral-400">불러오는 중...</p>
      )}

      {recipes !== null && recipes.length === 0 && (
        <div className="mx-auto max-w-lg space-y-2 rounded-lg border border-neutral-800 bg-neutral-900/40 p-6 text-center">
          <p className="text-sm text-neutral-300">라이브러리가 비어있습니다.</p>
          <p className="text-xs text-neutral-500">
            <code className="text-neutral-400">.pengz</code> 파일을 열거나, [새 앱 추가]
            버튼을 눌러서 라이브러리 등록이 가능합니다.
          </p>
        </div>
      )}

      {recipes !== null && recipes.length > 0 && (
        <ul
          className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3"
          // 개별 카드가 아니라 그리드 전체에서 "가장 가까운 카드"를 계산 — 카드 사이
          // gap 위에서 놓아도(어떤 카드 rect 에도 안 걸려도) 무시되지 않는다.
          onDragOver={(e) => {
            if (!draggedId) return;
            e.preventDefault();
            const nearest = findNearestRecipeId(e.clientX, e.clientY);
            if (nearest && nearest !== draggedId) setDragOverId(nearest);
          }}
          onDrop={(e) => {
            e.preventDefault();
            if (draggedId) {
              const nearest = findNearestRecipeId(e.clientX, e.clientY);
              if (nearest) handleReorderCards(draggedId, nearest);
            }
            setDraggedId(null);
            setDragOverId(null);
          }}
        >
          {recipes.map((recipe) => (
            <li
              key={recipe.id}
              ref={(el) => {
                if (el) cardRefs.current.set(recipe.id, el);
                else cardRefs.current.delete(recipe.id);
              }}
              // 선택 모드에선 드래그 재정렬을 끈다 — 같은 카드에서 "드래그해서 순서
              // 바꾸기"와 "클릭해서 선택"이 동시에 활성화되면 제스처가 헷갈린다.
              draggable={!selectionMode}
              onDragStart={() => setDraggedId(recipe.id)}
              onDragEnd={() => {
                setDraggedId(null);
                setDragOverId(null);
              }}
              className={`select-none rounded-lg transition-shadow ${
                selectionMode ? "" : "active:cursor-grabbing"
              } ${draggedId === recipe.id ? "opacity-40" : ""} ${
                dragOverId === recipe.id ? "ring-2 ring-blue-400/50" : ""
              }`}
            >
              <AppCard
                recipe={recipe}
                onInstall={(r) => void handleInstall(r)}
                installing={installingId === recipe.id}
                onCancelInstall={handleCancelInstall}
                onLaunch={(r) => void handleLaunch(r)}
                launching={launchingId === recipe.id}
                onRemove={() => handleRemove(recipe)}
                onDelete={() => handleDelete(recipe)}
                onReinstall={() => handleReinstall(recipe)}
                onOpenFolder={() => handleOpenFolder(recipe)}
                onLinkFolder={() => setPendingLocalRootOverride(recipe)}
                onEdit={() => void handleEditRequest(recipe.id)}
                onExport={() => void handleExport(recipe)}
                statusRefreshKey={statusVersion}
                selectionMode={selectionMode}
                selected={selectedIds.has(recipe.id)}
                onToggleSelect={() => toggleSelected(recipe.id)}
              />
            </li>
          ))}
        </ul>
      )}

      <ThirdPartyInstallDialog
        appId={
          pendingInstall?.recipe.launch.kind === "third_party_app_launch"
            ? pendingInstall.recipe.launch.app_id
            : null
        }
        onInstalled={() => void handleThirdPartyInstalled()}
        onCancel={() => setPendingInstall(null)}
      />

      <OptionalGroupsDialog
        recipe={pendingOptionalGroups}
        onConfirm={(groups) => void handleOptionalGroupsConfirmed(groups)}
        onCancel={() => setPendingOptionalGroups(null)}
      />

      <OverrideConflictDialog
        recipe={pendingOverrideConflicts?.recipe ?? null}
        conflicts={pendingOverrideConflicts?.conflicts ?? []}
        onConfirm={(resolutions) => void handleOverrideConflictsResolved(resolutions)}
        onCancel={() => setPendingOverrideConflicts(null)}
      />

      <ArchiveConflictDialog
        recipe={pendingArchiveConflicts?.recipe ?? null}
        archives={pendingArchiveConflicts?.archives ?? []}
        onConfirm={(resolved) => void handleArchiveConflictsResolved(resolved)}
        onCancel={() => setPendingArchiveConflicts(null)}
      />

      <LocalRootOverrideDialog
        recipe={pendingLocalRootOverride}
        onClose={() => setPendingLocalRootOverride(null)}
      />

      <DeleteInstalledDataDialog
        recipe={pendingDelete}
        onConfirmAll={handleDeleteAllConfirmed}
        onConfirmGroups={handleDeleteGroupsConfirmed}
        onCancel={() => setPendingDelete(null)}
      />

      {confirmDialog}

      {dialogOpen && (
        <RecipeEditDialog
          recipe={editingRecipe}
          existingIds={recipes?.map((r) => r.id) ?? []}
          onSave={handleSaveEdit}
          onCancel={closeDialog}
        />
      )}

      {toast && (
        <div
          className={`fixed bottom-6 right-6 max-w-sm cursor-pointer break-words rounded-lg border px-4 py-3 text-sm shadow-lg ${
            toast.kind === "error"
              ? "border-red-900/60 bg-red-950/80 text-red-200"
              : "border-emerald-900/60 bg-emerald-950/80 text-emerald-200"
          }`}
          onClick={() => setToast(null)}
          role="status"
        >
          <p>{toast.message}</p>
          {toast.kind === "error" && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void handleCopyToastMessage(toast.message);
              }}
              className="mt-2 cursor-pointer text-xs text-red-300 underline-offset-2 hover:text-red-100 hover:underline"
            >
              {copiedToast ? "복사됨" : "에러 복사"}
            </button>
          )}
        </div>
      )}
    </div>
  );
}

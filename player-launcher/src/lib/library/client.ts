// `commands/library.rs` 의 Tauri 커맨드 typed wrapper. 옛 `lib/psp/client.ts` 대체.
//
// 카탈로그/인스턴스 fetch 가 없어져 TTL 캐시(`lib/psp/cache.ts`)도 함께 사라짐 —
// `library_list` 는 로컬 파일 읽기라 매번 다시 불러도 비용이 낮음.

import { invoke } from "@tauri-apps/api/core";
import type {
  ArtifactVerification,
  ConfigFileFormat,
  ConfigPatch,
  ImportPreview,
  InstallDiagnostic,
  InstallOutcome,
  InstallStatus,
  LaunchOutcome,
  Recipe,
} from "./types";

export function libraryList(): Promise<Recipe[]> {
  return invoke<Recipe[]>("library_list");
}

export function libraryUpsert(recipe: Recipe): Promise<void> {
  return invoke<void>("library_upsert", { recipe });
}

/** 지금 설정된 로컬 경로 오버라이드 — 없으면 `null`(자동 관리 경로 사용 중). 다이얼로그를
 * 다시 열어 현재 값을 미리 채워 보여줄 때 씀. */
export function libraryGetLocalRootOverride(id: string): Promise<string | null> {
  return invoke<string | null>("library_get_local_root_override", { id });
}

/** 포터블 앱을 이미 다른 경로에 설치해둔 경우 등 — 로컬 전용 루트 오버라이드.
 * `root: null` 이면 해제(자동 관리 경로로 복귀). */
export function librarySetLocalRootOverride(id: string, root: string | null): Promise<void> {
  return invoke<void>("library_set_local_root_override", { id, root });
}

/** 지금 확정된 선택적 그룹 선택 — `null`이면 아직 한 번도 확인 안 함. 선택 다이얼로그를
 * 다시 열어 현재 선택을 미리 채워 보여줄 때 씀. */
export function libraryGetSelectedOptionalGroups(id: string): Promise<string[] | null> {
  return invoke<string[] | null>("library_get_selected_optional_groups", { id });
}

/** 선택적 그룹 선택을 확정/변경 — 확인 다이얼로그에서 체크박스를 확정한 뒤 호출.
 * 이후 `libraryInstall`이 이 선택 기준으로 재조정(켠 그룹은 복원, 끈 그룹은 삭제)한다. */
export function librarySetSelectedOptionalGroups(id: string, groups: string[]): Promise<void> {
  return invoke<void>("library_set_selected_optional_groups", { id, groups });
}

export function libraryRemove(id: string): Promise<boolean> {
  return invoke<boolean>("library_remove", { id });
}

/** 라이브러리 카드 순서(드래그 재배치) 저장 — `ids`는 화면에 보여줄 순서 그대로
 * 전체 id 목록. */
export function libraryReorder(ids: string[]): Promise<void> {
  return invoke<void>("library_reorder", { ids });
}

/** `.pengz` 파일로 내보내기. ids 가 비면 백엔드가 에러. `savePath`는
 * `@tauri-apps/plugin-dialog`의 저장 다이얼로그로 미리 받아온 경로. */
export function libraryExportFile(ids: string[], savePath: string): Promise<void> {
  return invoke<void>("library_export_file", { ids, savePath });
}

/** `.pengz` 파일 경로 미리보기 — 스토어 안 바꿈. */
export function libraryPreviewImportFile(path: string): Promise<ImportPreview> {
  return invoke<ImportPreview>("library_preview_import_file", { path });
}

/** 사용자가 confirm 한 뒤 호출 — 실제로 라이브러리에 반영. */
export function libraryConfirmImportFile(path: string): Promise<string[]> {
  return invoke<string[]>("library_confirm_import_file", { path });
}

/** 더블클릭으로 콜드 스타트(이 앱 자체가 `.pengz` 경로를 인자로 받으며 새로 뜬 경우)
 * 됐을 때 잡아둔 파일 경로 — 프론트엔드가 mount 직후 1회 조회해서 소비한다(다음
 * 호출은 `null`). 핫 스타트(이미 실행 중일 때 더블클릭)는 이 커맨드가 아니라
 * `"pengz-file-opened"` 이벤트로 옴 — `App.tsx` 참고. */
export function takePendingPengzFile(): Promise<string | null> {
  return invoke<string | null>("take_pending_pengz_file");
}

/** 처음 설치든, 이미 설치된 레시피의 변경분 반영("업데이트")이든 지금 레시피와 실제
 * 설치 상태가 다른 스텝만 적용하는 같은 커맨드 — "설치" 버튼은 항상 이것만 호출한다. */
export function libraryInstall(recipe: Recipe): Promise<InstallOutcome> {
  return invoke<InstallOutcome>("library_install", { recipe });
}

/** 진행 중인 설치/업데이트를 취소 — 협조적 취소라 즉시 멈추진 않고, 실행 중인 다운로드/
 * 압축 해제가 다음 청크/엔트리를 처리하기 전에 스스로 멈춘다. 반환값 = 취소 대상을
 * 실제로 찾았는지(이미 끝났으면 false). */
export function libraryCancelInstall(recipeId: string): Promise<boolean> {
  return invoke<boolean>("library_cancel_install", { recipeId });
}

/** "브라우저로 열어서 받기" 압축이 다운로드 폴더 감시로 자동으로 안 잡힐 때(다른
 * 폴더에 저장 등) 사용자가 받은 파일을 직접 지정 — 이 레시피의 스크래치 폴더로
 * 복사한 뒤 그 자리에서 바로 `verification`과 대조한다(안 맞으면 throw — 자동 감시와
 * 달리 사용자가 명시적으로 고른 파일이라 즉시 알려줘야 함). `verification`은
 * `install:browser-download-waiting` 이벤트로 받은 값을 그대로 넘긴다. */
export function libraryStageManualArchiveFile(
  recipeId: string,
  path: string,
  verification: ArtifactVerification,
): Promise<void> {
  return invoke<void>("library_stage_manual_archive_file", { recipeId, path, verification });
}

/** 레시피 실행 — 설치는 안 함(설치 안 돼 있으면 에러). per-launch confirm 없음 —
 * 라이브러리에 있다는 것 자체가 신뢰 표시. */
export function libraryLaunch(recipe: Recipe): Promise<LaunchOutcome> {
  return invoke<LaunchOutcome>("library_launch", { recipe });
}

/** 설치/업데이트가 필요한지 조회만(부작용 없음) — 카드에 "미설치"/"업데이트 필요"
 * 뱃지를 보여주기 위함. */
export function libraryInstallStatus(recipe: Recipe): Promise<InstallStatus> {
  return invoke<InstallStatus>("library_install_status", { recipe });
}

/** "업데이트 필요"가 왜 뜨는지 — 어느 항목(압축/오버라이드)이 아직 반영된 적 없는지의
 * 목록. 카드 렌더링마다 부르지 말고 사용자가 뱃지를 눌렀을 때만. */
export function libraryInstallDiagnostics(recipe: Recipe): Promise<InstallDiagnostic[]> {
  return invoke<InstallDiagnostic[]>("library_install_diagnostics", { recipe });
}

/** 레시피가 로컬에 설치한 폴더를 OS 파일 탐색기로 연다. */
export function libraryOpenFolder(recipe: Recipe): Promise<void> {
  return invoke<void>("library_open_folder", { recipe });
}

/** 설치된 데이터를 삭제 — **라이브러리 항목은 남긴다**("라이브러리에서 제거"는
 * 목록에서만 뺌, 이건 반대로 설치 데이터만 지우고 목록엔 그대로 남겨서 나중에 다시
 * 설치할 수 있게 함). `groups`를 생략하면 전체 삭제, id 목록을 주면 그 선택적
 * 그룹들만 부분 삭제(베이스 + 다른 그룹은 유지). 로컬 루트 오버라이드가 설정된
 * 항목은 백엔드가 거부한다. */
export function libraryDeleteInstalledData(recipe: Recipe, groups?: string[]): Promise<void> {
  return invoke<void>("library_delete_installed_data", { recipe, groups: groups ?? null });
}

/** 레시피 편집 화면의 "파일에서 불러오기" — 로컬 설정 파일을 통째로 읽어
 * `OverrideContent::ConfigPatch.patch`에 채울 JSON 값으로 파싱. */
export function readConfigFileAsPatch(path: string, format: ConfigFileFormat): Promise<ConfigPatch> {
  return invoke<ConfigPatch>("read_config_file_as_patch", { path, format });
}

/** 레시피 편집 화면의 "폴더 불러오기" — 로컬 폴더를 재귀적으로 훑어서 상대경로
 * 전부(정렬됨)를 나열. `Recipe.files` 화이트리스트를 손으로 수백 개 타이핑하지 않고
 * 채우기 위함. */
export function scanFolderRelativePaths(root: string): Promise<string[]> {
  return invoke<string[]>("scan_folder_relative_paths", { root });
}

/** 레시피 편집 화면의 "파일에서 계산" — 로컬 아티팩트 파일 하나를 골라 그 자리에서
 * SHA256을 계산해 `ArtifactVerification.hash`에 채워준다. `sha256sum` 같은 외부
 * 도구를 따로 안 써도 되게 하기 위함. */
export function computeFileSha256(path: string): Promise<string> {
  return invoke<string>("compute_file_sha256", { path });
}

/** 등록된 third-party app descriptor id 목록(로컬 파일, 링크 임포트로 채워짐) —
 * 레시피 편집 화면의 "대상 서드파티 앱" 드롭다운이 자유 텍스트 대신 이 목록에서
 * 고르게 한다. */
export function listThirdPartyAppIds(): Promise<string[]> {
  return invoke<string[]>("list_third_party_app_ids");
}

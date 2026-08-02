// 0.2.0 앱 라이브러리 타입 — Rust `shared/src/library/recipe.rs` + `import.rs` +
// `player-launcher/src-tauri/src/commands/library.rs` 와 수동 동기 유지.
//
// v8 스키마 — 설치는 다운로드(`archives`)와 오버라이드(`files`) 두 종류의 사실만
// 있다("스텝" 개념 폐기). 실행 순서는 항상 고정: 압축 전부 다운로드+해제+화이트리스트
// 정리 → 그다음 오버라이드 전부 적용. 대상 루트(App 전용 폴더냐 third-party 앱
// 데이터 영역이냐)는 `archives`/`files` 항목마다 따로 고르지 않고 `Recipe.launch`
// 하나가 결정한다 — 실제 데이터에서 이 둘은 항상 1:1이었다. third-party app(Prism 등)
// 지원은 전용 타입이 아니라 `app_id: string` 데이터 참조로 표현된다.

export interface Recipe {
  id: string;
  name: string;
  recipe_info: RecipeInfo;
  archives: ArchiveExtraction[];
  files: RecipeFile[];
  optional_groups: OptionalGroup[];
  folder_rules: FolderRule[];
  launch: LaunchAction;
}

// `Recipe.folder_rules` 항목 하나 — 화이트리스트 정리(`Recipe.files` 기준 pruning)의
// 기본 동작을 폴더 단위로 완화하는 예외 선언. `path`는 `RecipeFile.path`와 같은 표현
// (대상 루트 기준 상대경로, 슬래시 구분)이고 `Recipe.folder_rules` 안에서 유일해야 함.
// 설치까지만 관여하고 설치 후 앱 사용으로 생기는 변화는 건드리지 않는다는 원칙에
// 따라 `filtered`도 이 규칙(경로+패턴) 자체가 바뀌지 않는 한 딱 1회만 정리한다 —
// 매 재설치마다 계속 강제하는 옵션은 없음.
export interface FolderRule {
  path: string;
  mode: FolderRuleMode;
}

// Rust `FolderRuleMode`는 `#[serde(tag = "kind")]`. `disallow_patterns`는 `patterns`로
// 들어온 허용을 좁히는 예외 — 명시적으로 선언된 `RecipeFile`은 절대 못 지운다
// (`recipe.rs`의 `FolderRuleMode::Filtered` 문서 참고).
export type FolderRuleMode =
  | { kind: "passthrough" }
  | { kind: "filtered"; patterns: string[]; disallow_patterns: string[] };

// `Recipe.optional_groups` 항목 하나 — 부분 설치 가능한 그룹의 선언(표시용 메타데이터
// 포함). 선택 여부 자체는 여기 없다(로컬 전용 상태 — `libraryGetSelectedOptionalGroups`
// 참고). `default_selected`는 확인 다이얼로그에 미리 체크될 값일 뿐, 사용자 확인 없이
// 자동으로 설치되지 않는다.
export interface OptionalGroup {
  // `label`에서 파생된 순수 참조 키(기술적 제약 없음) — 생성 시점에 한 번 채워지고
  // 그 뒤로 `label`이 바뀌어도 고정 유지(이미 설치된 사용자의 선택 상태가 이 값
  // 기준으로 저장되므로).
  id: string;
  label: string;
  default_selected: boolean;
}

export interface RecipeInfo {
  icon_url?: string | null;
  /** 카드 배경 이미지 — 순수 표시용(검증 대상 아님, `icon_url`과 동일 취급). */
  background_url?: string | null;
}

export type ArtifactVerification = { kind: "sha256"; hash: string };

// Rust `FileContent` 는 `#[serde(tag = "encoding")]`.
export type FileContent =
  | { encoding: "text"; content: string }
  | { encoding: "base64"; data: string };

// 순수 태그 — 실제 파싱/patch 적용은 Rust 쪽 `commands/config_patch.rs`가 포맷별로
// 분기해서 처리한다. 새 포맷이 늘어도 이 union에 문자열 하나 추가되는 것 외엔 프론트
// 코드(RecipeEditDialog 등)가 안 바뀐다 — `patch`는 항상 포맷 무관 JSON.
export type ConfigFileFormat = "ini" | "json" | "toml";

// `OverrideContent::ConfigPatch.patch` — ini/toml 은 `{"섹션": {"키": 값}}`, json 은 파일
// 최상위 구조 그 자체. 포맷 무관 공통 표현이라 구체 타입을 두지 않는다.
export type ConfigPatch = Record<string, unknown>;

// 압축을 받아서 어디에 풀지 — 다운로드는 "무엇을 어디서 받을지"만 정하고, 압축 안의
// 개별 파일이 정확히 뭔지는 모른다. 실행 시점에 `Recipe.files` 화이트리스트가 담당 —
// 선언 안 된 파일은 즉시 삭제된다(예외 없음).
export interface ArchiveExtraction {
  // 직접 다운로드 링크 또는 "사람이 눌러서 받아야 하는" 페이지(구글 드라이브 공유
  // 페이지 등) 둘 다 될 수 있다 — 구분해서 선언할 필요 없이 자동 감지: PengPort가
  // 먼저 직접 받아보고, 응답이 실제 파일이 아니라 페이지(HTML)로 판명되면 그때
  // 기본 브라우저로 열어 사람이 받게 한 뒤 다운로드 폴더를 감시해서 `verification`
  // 해시와 일치하는 파일을 찾는다.
  url: string;
  // 카드 목록에 표시할 이름 — 없으면 url 마지막 경로 조각에서 유도(archiveDisplayName).
  // 단축 URL(short.example/... 등)은 그 조각이 알아보기 힘들어서 직접 지정하는 용도.
  label?: string | null;
  verification: ArtifactVerification;
  // 다운로드+적용 순서 — `Recipe.archives` 안에서 유일해야 한다(백엔드가 검증). 두
  // 압축이 같은 목적지에 겹치는 파일을 만들면 이 값이 더 큰 쪽이 최종적으로 남는다.
  order: number;
  extract_to: string;
  // `RecipeFile.optional_group`과 같은 개념 — 없으면 항상 다운로드, 있으면 그 그룹이
  // 선택됐을 때만 다운로드(압축 자체가 특정 선택 그룹 전용인 경우).
  optional_group?: string | null;
  // 있으면 압축이 아니라 검증된 단일 파일 하나를 이 이름으로 `extract_to` 밑에 배치.
  raw_filename?: string | null;
  // 압축 내부 구조가 평평해서 `extract_to` 하나로는 표현 못 하는 개별 파일 재배치 —
  // "이 압축 안의 이 파일은 저 위치로 간다"는 포트포워딩처럼 항상 명시적(자동
  // 추측 없음). 대부분의 파일은 `extract_to`가 처리하고, 이건 그중 예외만 콕 집어
  // 다시 보낼 때 쓴다.
  path_overrides?: PathOverride[];
}

// [`ArchiveExtraction.path_overrides`] 항목 하나. `from`은 압축 안 경로(압축을 그대로
// 열었을 때 보이는 경로), `to`는 대상 루트 기준 최종 경로(`RecipeFile.path`와 같은
// 표현 — 슬래시 구분).
export interface PathOverride {
  from: string;
  to: string;
}

// `RecipeFile::override_content` — 파일에 실제로 어떤 내용을 반영할지.
export type OverrideContent =
  | { kind: "literal"; content: FileContent }
  | { kind: "config_patch"; format: ConfigFileFormat; patch: ConfigPatch };

// 레시피가 아는 파일 하나 — 위치(`path`)가 유일한 진실이고, 있다면 그 위에 덮어씌울
// 내용까지 이 한 항목이 전부 갖고 있다. `override_content` 가 없으면 압축 해제 결과
// 그대로(화이트리스트 멤버로만 존재).
export interface RecipeFile {
  path: string;
  override_content?: OverrideContent | null;
  // 없으면 항상 필수. 있으면 `Recipe.optional_groups`의 그 id 그룹에 속함 — 사용자가
  // 그 그룹을 선택했을 때만 화이트리스트에 포함된다.
  optional_group?: string | null;
}

export type LaunchAction =
  | { kind: "spawn_process"; entry_point: string; entry_args: string[] }
  | { kind: "third_party_app_launch"; app_id: string };

export interface ImportPreviewItem {
  id: string;
  name: string;
  icon_url?: string | null;
  already_in_library: boolean;
}

// `import::ImportPreviewThirdPartyApp`. 레시피(`ImportPreviewItem`)와 달리 `icon_url`이
// 없다 — third-party app 은 실행 위치 데이터이지 표시용 라이브러리 항목이 아니다.
export interface ImportPreviewThirdPartyApp {
  id: string;
  label: string;
  already_registered: boolean;
}

export interface ImportPreview {
  items: ImportPreviewItem[];
  third_party_apps: ImportPreviewThirdPartyApp[];
}

// `commands/library.rs` 의 `LaunchOutcome` (serde internally-tagged).
export type LaunchOutcome =
  | { kind: "launched" }
  | { kind: "third_party_app_missing"; app_id: string };

// `commands/library.rs` 의 `InstallOutcome` — "설치" 버튼이 부르는 `library_install`의
// 결과(처음 설치든 이미 설치된 레시피의 변경분 반영이든 같은 커맨드). `updated === 0`
// 이면 이미 최신 상태라는 뜻.
export type InstallOutcome =
  | { kind: "completed"; updated: number; total: number }
  | { kind: "using_local_override" }
  | { kind: "third_party_app_missing"; app_id: string }
  // `Recipe.optional_groups`가 있는데 아직 선택을 확인 안 함 — 선택 다이얼로그
  // 표시 후 확정하고 재시도해야 한다(third_party_app_missing과 같은 재시도 패턴).
  | { kind: "needs_optional_group_selection" }
  // 사용자가 `libraryCancelInstall`로 도중에 멈춤 — 에러가 아니라 정상적인 사용자
  // 의사결정. 이미 적용된 항목은 되돌리지 않고, 다음 설치 때 마커 기준으로 이어서
  // 진행한다(크래시 복구와 같은 방식).
  | { kind: "cancelled" };

// `commands/library.rs` 의 `InstallStatus` — 카드에 "미설치"/"업데이트 필요" 뱃지를
// 보여주기 위한 조회 전용(부작용 없음). 원장(마커) 기반 판정 — 지금 실제 파일 내용을
// 레시피 선언값과 비교하지 않는다(설치 이후 앱 사용으로 생기는 정상적인 변화까지
// 업데이트 필요로 오판하지 않기 위함).
export type InstallStatus =
  | { kind: "up_to_date" }
  | { kind: "not_installed" }
  | { kind: "update_available"; pending: number; total: number }
  | { kind: "using_local_override" }
  | { kind: "needs_optional_group_selection" };

// `commands/library.rs` 의 `InstallDiagnostic` — "업데이트 필요" 뱃지를 눌렀을 때
// 온디맨드로 조회하는, 어느 항목이 아직 반영된 적 없는지의 목록. 항목 내용의 "어느
// 부분이 다른지"까지는 안 보여준다 — 설치 이후 앱 사용으로 생기는 정상적인 변화와
// 진짜 레시피 변경을 실제 파일 비교로는 구분할 수 없기 때문("아직 반영된 적 없다"는
// 원장 사실만 정직하게 보여줌).
export type InstallDiagnostic =
  // `missing_paths` — snake_case 그대로(이 타입은 derive(Serialize)라 프로젝트 전반의
  // camelCase 변환이 없음, `ArchiveExtraction` 등 다른 derive 타입과 동일 컨벤션).
  | { kind: "archive_pending"; url: string; missing_paths: string[] }
  | { kind: "file_pending"; path: string }
  | { kind: "needs_optional_group_selection" };

// `shared::actions::third_party_app` 의 타입들 — 서드파티 앱(예: Prism Launcher) 하나를
// 코드가 아니라 데이터로 표현. `ThirdPartyApps.tsx`(설정 화면)의 등록/편집 폼이 다룬다.

// Rust `ReadinessSignal` 는 `#[serde(tag = "kind")]` — variant 하나뿐(모듈 설명 참고 —
// 다른 앱이 자체 신호를 주면 그때 variant 추가, 억지로 넓히지 않음).
export type ReadinessSignal = { kind: "child_process_window"; cmdline_contains: string };

// Rust `DownloadStrategy` 는 `#[serde(tag = "kind")]`.
export type DownloadStrategy =
  | { kind: "static_url"; url: string; verification: ArtifactVerification }
  | { kind: "github_latest_release"; repo: string; asset_name_pattern: string };

// Rust `ThirdPartyAppDescriptor` — 서드파티 앱 하나의 전체 정의. 시스템에 이미 설치된
// 위치는 `exe_filename` 하나만으로 찾는 고정 알고리즘(`detect_third_party_app`, 앱별
// 설정 불필요)이라 여기엔 대응하는 필드가 없다.
export interface ThirdPartyAppDescriptor {
  id: string;
  label?: string | null;
  exe_filename: string;
  download_strategy?: DownloadStrategy | null;
  post_download_marker_files: string[];
  instances_subfolder?: string | null;
  system_appdata_folder_name?: string | null;
  readiness_signal?: ReadinessSignal | null;
  launch_args_template: string[];
}

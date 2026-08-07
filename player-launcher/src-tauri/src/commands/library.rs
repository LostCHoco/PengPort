//! 앱 라이브러리(레시피 목록) + `.pengz` 파일 임포트/내보내기 + 실행 Tauri 커맨드. v8 스키마.
//!
//! 설치는 다운로드([`Recipe::archives`])와 오버라이드([`Recipe::files`]) 두 종류의
//! 사실만 있다 — "스텝" 개념(순서 있는 이종 명령어 목록) 자체를 없앴다. 실행 순서는
//! 항상 고정: **압축 전부 다운로드+검증+해제(+화이트리스트 정리) → 그다음 오버라이드
//! 전부 적용**. 압축 해제 직후 그 안에서 실제로 나온 파일들을 [`Recipe::files`]와
//! 대조해서, 레시피가 모르는 파일은 즉시 삭제한다(예외 없음 — 다운로드 링크가 바뀌어
//! 다른 미러를 쓰게 되거나 압축이 여러 개로 쪼개져도 "레시피가 아는 파일만 남는다"는
//! 불변식은 항상 지켜짐).
//!
//! 설치/업데이트 필요 여부는 **원장(마커) 기반**이다 — "이 정확한 압축/오버라이드를
//! 성공적으로 적용한 적이 있는가"만 추적하고, 지금 실제 파일 내용을 레시피 선언값과
//! 실시간으로 비교하지 않는다. 설치 이후 앱을 사용하면서 생기는 정상적인 변화(캐시,
//! 사용자가 앱 안에서 바꾼 설정 등)까지 "업데이트 필요"로 오판하는 걸 막기 위함 —
//! 실제로 겪은 버그(Prism이 instance.cfg/mmc-pack.json에 런타임 필드를 계속 추가하는
//! 걸 "레시피와 다르다"고 오판했었음)에서 나온 설계.
//!
//! 프론트엔드 흐름:
//! 1. `.pengz` 파일 열기(더블클릭 또는 파일 선택) → `commands::file_import::
//!    library_preview_import_file` 호출 → confirm 다이얼로그에 결과 표시(1회만).
//! 2. 사용자 확인 → `library_confirm_import_file` 호출 → 라이브러리에 반영.
//! 3. 라이브러리 화면은 [`library_list`]로 flat 렌더링, 항목 제거는 [`library_remove`].
//! 4. "이 항목들을 내보내기" → `library_export_file`로 `.pengz` 파일 생성.
//! 5. **설치/업데이트/실행 세 버튼**([`library_install`] / [`library_launch`], "설치"와
//!    "업데이트"는 같은 커맨드 — 원장에 없는 항목만 적용하는 같은 동작). 실행은 설치를
//!    자동으로 하지 않는다 — 설치 안 됐으면 명확한 에러로 알려줄 뿐(사용자가
//!    설치/업데이트/실행을 각각 명시적으로 통제).

use std::cell::Cell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use pengport_shared::actions::{validate_recipe, ActionContext};
use pengport_shared::library::{
    ArchiveExtraction, ArtifactVerification, FileContent, FolderRuleMode, LaunchAction,
    LibraryStore, OptionalGroup, OverrideContent, PathOverride, Recipe, RecipeFile, RecipeInfo,
    Sha256Verifier, ThirdPartyAppStore,
};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tauri_plugin_opener::OpenerExt;

/// 진행률/스텝 이벤트 과다 발생(64KB 청크마다 emit 하면 3GB 파일 기준 5만 개 가까운
/// IPC 메시지)을 막는 시간 기준 스로틀. `Cell`로 내부 가변성만 필요 — 단일 스레드
/// 안에서 `FnMut` 클로저 캡처로만 쓰인다(스레드 간 공유 없음).
struct Throttle {
    last: Cell<Instant>,
    min_interval: Duration,
}

impl Throttle {
    fn new(min_interval: Duration) -> Self {
        Self {
            last: Cell::new(Instant::now() - min_interval),
            min_interval,
        }
    }

    /// 마지막 허용 이후 `min_interval` 이상 지났으면 `true`(호출자는 이번엔 emit 해도
    /// 됨) 를 반환하고 내부 타이머를 갱신한다.
    fn allow(&self) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last.get()) >= self.min_interval {
            self.last.set(now);
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// 설치 취소 — 레시피 id 별로 `AtomicBool` 플래그를 두고, 다운로드/압축 해제 루프가
// 매 청크/엔트리마다(이미 진행률 콜백을 부르는 지점이라 추가 비용이 거의 없음)
// 확인한다. `library_cancel_install`이 플래그만 켜면, 실행 중인 스레드가 다음 체크
// 시점에 스스로 멈춘다 — 강제 kill 이 아니라 협조적 취소(cooperative cancellation).
// ---------------------------------------------------------------------------

fn install_cancel_flags() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static MAP: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// `reconcile_install` 시작 시 새 플래그를 등록 — 이전 시도의 낡은 플래그를 재사용하지
/// 않도록 항상 새로 만든다(이전 설치가 취소된 채 끝났어도 다음 설치는 깨끗하게 시작).
fn register_install_cancel_flag(recipe_id: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    install_cancel_flags()
        .lock()
        .unwrap()
        .insert(recipe_id.to_string(), flag.clone());
    flag
}

/// `reconcile_install`이 어떤 경로로 끝나든(성공/실패/취소) 레지스트리에서 자기
/// 항목을 지운다 — Rust 에 try/finally 가 없어 RAII(Drop) 로 대체(`TempFileGuard`와
/// 같은 패턴).
struct InstallCancelGuard(String);

impl Drop for InstallCancelGuard {
    fn drop(&mut self) {
        install_cancel_flags().lock().unwrap().remove(&self.0);
    }
}

/// 다운로드/압축 해제 루프 안에서 부르는 취소 확인 — 취소됐으면 이 문자열 그대로
/// `Err`. `library_install`이 이 정확한 문자열을 보고 `InstallOutcome::Cancelled`로
/// 바꿔치기한다(일반 에러 토스트가 아니라 "취소됨"으로 조용히 보여야 하므로).
pub(super) const INSTALL_CANCELLED_SENTINEL: &str = "__pengport_install_cancelled__";

pub(super) fn check_cancelled(flag: &AtomicBool) -> Result<(), String> {
    if flag.load(Ordering::Relaxed) {
        Err(INSTALL_CANCELLED_SENTINEL.to_string())
    } else {
        Ok(())
    }
}

/// 진행 중인 설치를 취소 — 플래그만 켜고 즉시 반환(실제로 멈추는 건 실행 중인
/// 스레드가 다음 체크 시점에 스스로 함). 반환값 = 취소 대상을 찾았는지 — 이미
/// 끝났거나 애초에 설치 중이 아니면 `false`(프론트가 "취소할 게 없었음"을 구분).
#[tauri::command]
pub fn library_cancel_install(recipe_id: String) -> bool {
    if let Some(flag) = install_cancel_flags().lock().unwrap().get(&recipe_id) {
        flag.store(true, Ordering::Relaxed);
        true
    } else {
        false
    }
}

/// 다운로드 임시 파일을 스코프를 벗어나는 모든 경로(성공/에러 조기 반환 포함)에서
/// 항상 정리 — Rust 에 try/finally 가 없어 RAII(Drop) 로 대체.
pub(super) struct TempFileGuard(pub(super) PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn allow_http() -> bool {
    cfg!(debug_assertions)
}

fn library_store_path() -> Result<PathBuf, String> {
    super::paths::app_data_root()
        .map(|d| d.join("library.json"))
        .ok_or_else(|| "app_data_root 미정 (%APPDATA% 환경변수 없음)".to_string())
}

pub(super) async fn load_library_store() -> Result<LibraryStore, String> {
    let path = library_store_path()?;
    tauri::async_runtime::spawn_blocking(move || {
        LibraryStore::load(path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

pub(super) async fn load_third_party_app_store() -> Result<ThirdPartyAppStore, String> {
    let path = third_party_apps_store_path()?;
    tauri::async_runtime::spawn_blocking(move || {
        ThirdPartyAppStore::load(path).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// 라이브러리 그리드가 실제로 필요로 하는 필드만 담은 가벼운 뷰 — `Recipe.archives`/
/// `Recipe.files`(개별 파일의 `override_content`에 수 MB짜리 리터럴 base64/텍스트
/// 콘텐츠가 실릴 수 있음)를 뺀다. 카드 렌더링·설치 상태 뱃지·third-party 앱 확인은
/// 전부 id 기반 커맨드(`library_install_status`/`library_install`/`library_launch` 등,
/// 백엔드가 디스크에서 직접 [`Recipe`] 전체를 읽음)로 처리되므로 그리드 단계에서
/// archives/files 내용이 필요 없다. 실제 콘텐츠가 필요한 유일한 자리(편집 다이얼로그를
/// 열 때)는 [`library_get`]으로 그 레시피 하나만 따로 받는다.
///
/// (2026-08, 실사용 리포트로 도입 — `library_list()`가 전체 `Recipe`를 그리드
/// 렌더링마다 IPC로 왕복시키던 게, base64 리터럴 오버라이드가 있는 레시피에서 라이브러리
/// 로딩을 눈에 띄게 느리게 만든 원인이었다.)
#[derive(Debug, Clone, Serialize)]
pub struct RecipeSummary {
    pub id: String,
    pub name: String,
    pub recipe_info: RecipeInfo,
    pub launch: LaunchAction,
    pub optional_groups: Vec<OptionalGroup>,
}

impl From<&Recipe> for RecipeSummary {
    fn from(r: &Recipe) -> Self {
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            recipe_info: r.recipe_info.clone(),
            launch: r.launch.clone(),
            optional_groups: r.optional_groups.clone(),
        }
    }
}

/// 라이브러리 전체 목록 — flat, 그루핑 없음. archives/files 콘텐츠를 뺀 가벼운 뷰만
/// 반환([`RecipeSummary`] 문서 참고). `local_root_override`는 애초에 [`Recipe`]에
/// 없는 로컬 전용 필드([`library_get_local_root_override`]로 별도 조회).
#[tauri::command]
pub async fn library_list() -> Result<Vec<RecipeSummary>, String> {
    let store = load_library_store().await?;
    Ok(store.list().iter().map(|e| RecipeSummary::from(&e.recipe)).collect())
}

/// [`library_list`]가 뺀 실제 콘텐츠(`archives`/`files[].override_content` 등)까지
/// 포함한 전체 [`Recipe`] 하나를 받는다 — 레시피 편집 다이얼로그를 열 때만 호출.
#[tauri::command]
pub async fn library_get(id: String) -> Result<Option<Recipe>, String> {
    let store = load_library_store().await?;
    Ok(store.get(&id).map(|e| e.recipe.clone()))
}

/// 레시피 직접 추가/갱신("직접 등록" 경로 — `.pengz` 파일 임포트가 아닌 경우). 구조 검증까지
/// 통과해야 저장 — junk 레시피가 라이브러리에 들어가는 걸 막는다.
#[tauri::command]
pub async fn library_upsert(recipe: Recipe) -> Result<(), String> {
    pengport_shared::validate_service_id(&recipe.id)
        .map_err(|e| format!("레시피 id 형식 오류 ({:?}): {e}", recipe.id))?;
    let ctx = ActionContext {
        allow_http: allow_http(),
    };
    validate_recipe(&recipe, &ctx).map_err(|e| e.to_string())?;

    let mut store = load_library_store().await?;
    tauri::async_runtime::spawn_blocking(move || {
        store.upsert(recipe);
        store.save().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// 지금 설정된 로컬 경로 오버라이드 — 없으면 `None`(자동 관리 경로 사용 중). UI 가
/// 다이얼로그를 다시 열어 현재 값을 미리 채워 보여줄 때 씀.
#[tauri::command]
pub async fn library_get_local_root_override(id: String) -> Result<Option<String>, String> {
    let store = load_library_store().await?;
    Ok(store
        .get(&id)
        .and_then(|e| e.local_root_override.clone())
        .map(|p| p.to_string_lossy().into_owned()))
}

/// 포터블 앱을 이미 다른 경로에 설치해둔 경우 등 — 로컬 전용 루트 오버라이드.
/// `root: None`이면 해제(자동 관리 경로로 복귀).
#[tauri::command]
pub async fn library_set_local_root_override(id: String, root: Option<String>) -> Result<(), String> {
    let root_path = root.map(PathBuf::from);
    let mut store = load_library_store().await?;
    tauri::async_runtime::spawn_blocking(move || {
        if !store.set_local_root_override(&id, root_path) {
            return Err(format!("레시피 '{id}' 를 찾을 수 없음"));
        }
        store.save().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// 지금 확정된 선택적 그룹 선택 — `None`이면 아직 한 번도 확인 안 함(설치 시 확인
/// 다이얼로그 필요). 확인 다이얼로그를 다시 열어 현재 선택을 미리 채워 보여줄 때 씀.
#[tauri::command]
pub async fn library_get_selected_optional_groups(id: String) -> Result<Option<Vec<String>>, String> {
    let store = load_library_store().await?;
    Ok(store
        .get(&id)
        .and_then(|e| e.selected_optional_groups.clone())
        .map(|s| s.into_iter().collect()))
}

/// 선택적 그룹 선택을 확정/변경 — 사용자가 확인 다이얼로그에서 체크박스를 확정한 뒤
/// 호출. 이후 [`library_install`]이 이 선택 기준으로 재조정(켠 그룹은 복원, 끈 그룹은
/// 삭제)한다.
#[tauri::command]
pub async fn library_set_selected_optional_groups(id: String, groups: Vec<String>) -> Result<(), String> {
    let mut store = load_library_store().await?;
    tauri::async_runtime::spawn_blocking(move || {
        let set: HashSet<String> = groups.into_iter().collect();
        if !store.set_selected_optional_groups(&id, Some(set)) {
            return Err(format!("레시피 '{id}' 를 찾을 수 없음"));
        }
        store.save().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// 레시피 편집 화면의 "폴더 불러오기" — 로컬 폴더(예: 직접 압축을 풀어본 결과물, 또는
/// 이미 설치된 인스턴스 폴더)를 재귀적으로 훑어서 상대경로 전부를 나열한다.
/// [`Recipe::files`] 화이트리스트를 손으로 수백 개 타이핑하지 않고 채우기 위함 —
/// 화이트리스트 정책 자체(예외 없이 항상 강제)는 완화하지 않고, 작성 부담만 도구로
/// 상쇄한다.
#[tauri::command]
pub async fn scan_folder_relative_paths(root: String) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = PathBuf::from(root);
        if !root.is_dir() {
            return Err(format!("폴더가 아님: {}", root.display()));
        }
        let mut files = Vec::new();
        collect_leaf_paths(&root, &[], &mut files)?;
        let mut rel: Vec<String> = files
            .into_iter()
            .filter_map(|p| p.strip_prefix(&root).ok().map(|r| r.to_path_buf()))
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        rel.sort();
        Ok(rel)
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// 레시피 편집 화면의 "파일에서 계산" — 이미 로컬에 갖고 있는 아티팩트 파일 하나를
/// 골라서 그 자리에서 SHA256을 계산해 [`ArtifactVerification`] 필드에 채워준다.
/// 레시피 작성자가 `sha256sum`/`Get-FileHash` 같은 외부 도구를 따로 안 써도 되게
/// 하기 위함 — 설치 시점 검증(`download_and_verify_to_file`)과 같은 청크 단위
/// 스트리밍(대용량 파일이라도 전체를 메모리에 안 올림)으로 [`Sha256Verifier`]를
/// 재사용한다.
#[tauri::command]
pub async fn compute_file_sha256(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Read;

        let mut file = std::fs::File::open(&path).map_err(|e| format!("파일 열기 실패: {e}"))?;
        let mut hasher = Sha256Verifier::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf).map_err(|e| format!("파일 읽기 실패: {e}"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher.finalize_hex())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// 레시피 편집 화면의 "파일에서 불러오기" — 로컬 파일을 통째로 base64로 인코딩해
/// 넘긴다(프론트가 UTF-8 텍스트로 디코드해 [`OverrideContent::Literal`]의
/// `FileContent::Text`를 채움 — 바이너리는 리터럴 override 대상이 아님, 위
/// `FileContent` 문서 참고). 프론트 JS가 파일을 직접 읽어 인코딩하면 수 MB 파일에서도
/// 거대 문자열 처리 자체가 무겁다(편집 다이얼로그 렌더링 지연의 원인이기도 했음,
/// 2026-08) — Rust 쪽에서 한 번에 처리해 완성된 문자열만 넘긴다.
#[tauri::command]
pub async fn read_file_base64(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let bytes = std::fs::read(&path).map_err(|e| format!("파일 읽기 실패: {e}"))?;
        Ok(STANDARD.encode(bytes))
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// "브라우저로 열어서 받기" 압축이 자동으로 안 잡힐 때의 수동 폴백 — 사용자가 고른
/// 파일을 이 레시피의 스크래치 "manual" 폴더로 복사한 뒤 **그 자리에서 바로
/// 검증**한다. `verification`은 프론트가 `install:browser-download-waiting`
/// 이벤트로 받아 쥐고 있던 값을 그대로 넘긴다(지금 대기 중인 압축이 어느 것인지는
/// 그 이벤트가 이미 알려줬으므로 여기서 다시 찾지 않는다).
///
/// 이전엔 여기서 검증을 안 하고 그냥 옮겨두기만 했다 — 이미 돌고 있는
/// [`obtain_via_browser_assisted_download`]의 감시 루프가 이 폴더도 지켜보다가
/// 해시가 맞으면 알아서 집어가고, 안 맞으면 조용히 계속 무시하는 걸로 충분하다고
/// 봤기 때문. 하지만 **사용자가 직접 "이 파일이다"라고 골라준 경우**는 자동 감시와
/// 신뢰 수준이 다르다 — 잘못된 파일을 골랐으면 그 자리에서 바로 알려줘야지, 30분
/// 타임아웃까지 조용히 기다리게 두면 사용자는 뭐가 잘못됐는지 전혀 알 수 없다
/// (실사용 중 발견 — 해시가 오래돼 안 맞는 레시피를 반복 시도해도 아무 반응이
/// 없어서 원인을 코드로 직접 찾아야 했음, 2026-08).
#[tauri::command]
pub async fn library_stage_manual_archive_file(
    recipe_id: String,
    path: String,
    verification: ArtifactVerification,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let tmp_dir = archive_tmp_dir(&recipe_id)?;
        let manual_dir = manual_staging_dir(&tmp_dir);
        std::fs::create_dir_all(&manual_dir)
            .map_err(|e| format!("임시 폴더 생성 실패 ({}): {e}", manual_dir.display()))?;
        let src = PathBuf::from(&path);
        let file_name = src
            .file_name()
            .ok_or_else(|| format!("파일 이름을 알 수 없음: {}", src.display()))?;
        let dest = manual_dir.join(file_name);
        std::fs::copy(&src, &dest)
            .map_err(|e| format!("파일 복사 실패 ({} → {}): {e}", src.display(), dest.display()))?;

        if let Err(e) = hash_file_and_verify(&dest, &verification) {
            let _ = std::fs::remove_file(&dest); // 안 맞는 사본을 스크래치 폴더에 남겨두지 않음.
            return Err(format!("선택한 파일이 이 항목과 일치하지 않습니다: {e}"));
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// 파일 하나를 청크 단위로 해시해 `verification`과 대조 — [`compute_file_sha256`]와
/// 달리 값을 돌려주지 않고 맞는지만 판정한다.
fn hash_file_and_verify(path: &Path, verification: &ArtifactVerification) -> Result<(), String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).map_err(|e| format!("파일 열기 실패: {e}"))?;
    let mut hasher = Sha256Verifier::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("파일 읽기 실패: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hasher.finish(verification).map_err(|e| e.to_string())
}

/// 레시피가 로컬에 설치한 폴더를 OS 파일 탐색기로 연다.
#[tauri::command]
pub async fn library_open_folder(id: String, app: tauri::AppHandle) -> Result<(), String> {
    let store = load_library_store().await?;
    let entry = store.get(&id).ok_or_else(|| format!("알 수 없는 레시피: {id}"))?;
    let root_override = entry.local_root_override.clone();
    let launch = entry.recipe.launch.clone();

    let dir = match root_override {
        Some(root) => root,
        None => resolve_target_root(&id, &launch)?,
    };
    if !dir.is_dir() {
        return Err(format!(
            "폴더가 아직 없습니다 — 한 번 실행해서 설치한 뒤 열 수 있습니다: {}",
            dir.display()
        ));
    }
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| format!("폴더 열기 실패: {e}"))
}

/// `root` 아래의 실행 파일에서 뜬 프로세스를 찾아 종료 — 설치 데이터 삭제 직전에
/// 호출한다. Windows 는 실행 중인 exe나 그 프로세스가 연 파일에 배타적 잠금을 걸어서,
/// 앱이 켜진 채로 지우면 `remove_dir_all`이 access denied(os error 5)로 실패한다
/// (2026-08 실사용 버그 리포트). `spawn_local_process`가 fire-and-forget이라 PID를
/// 따로 기억해두지 않으므로, 지금 떠 있는 프로세스를 경로 기준으로 다시 찾아낸다 —
/// 이 방식은 `SpawnProcess`/`ThirdPartyAppLaunch` 둘 다 대상 실행 파일이 항상
/// `root` 밑에 있다는 사실만 쓰므로 launch 종류를 안 가린다.
/// `terminate_processes_under`의 판정만 떼어낸 순수 함수 — 실제 프로세스 목록
/// 없이도 고정 fixture 경로로 결정론적 테스트 가능("판단 로직은 항상 fixture로
/// 영구 테스트" 원칙, `terminate_processes_under` 자체는 실제 OS 프로세스에
/// 의존해 테스트하지 않는다).
fn exe_under_root(exe: Option<&Path>, root: &Path) -> bool {
    exe.is_some_and(|exe| exe.starts_with(root))
}

fn terminate_processes_under(root: &Path) -> Result<(), String> {
    use sysinfo::{ProcessesToUpdate, System};

    let Ok(root) = root.canonicalize() else {
        return Ok(()); // 루트 자체가 없으면 지울 것도, 잠글 프로세스도 없음.
    };

    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All);

    let mut targets = Vec::new();
    for (pid, process) in sys.processes() {
        let exe = process.exe().and_then(|exe| exe.canonicalize().ok());
        if exe_under_root(exe.as_deref(), &root) {
            process.kill();
            targets.push(*pid);
        }
    }
    if targets.is_empty() {
        return Ok(());
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        sys.refresh_processes(ProcessesToUpdate::All);
        if targets.iter().all(|pid| sys.process(*pid).is_none()) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(
                "실행 중인 앱을 종료하지 못했습니다 — 앱을 직접 닫고 다시 시도해주세요".to_string(),
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// 설치된 데이터를 삭제 — **라이브러리 항목은 그대로 남긴다**("라이브러리에서
/// 제거"와는 별개 동작; 그건 목록에서만 뺌, 이건 설치 데이터만 지움).
///
/// `groups`가 `None`이면 대상 루트 폴더 + `.pengport-markers` 전체 삭제(기존 동작) —
/// 삭제 후엔 마커도 없으므로 카드는 자연히 "미설치"로 돌아가고 선택적 그룹 선택도
/// "아직 확인 안 함"으로 초기화된다. `groups`에 id 목록이 있으면 **그 그룹들만**
/// 지운다 — 각 그룹에 속한 압축의 `dest_dir`와 마커만 삭제하고, 나머지(베이스 +
/// 다른 그룹)는 그대로 둔다. 설치 쪽은 선택 다이얼로그에서 그룹을 끄면 이미 이렇게
/// 부분 삭제가 됐는데("선택 해제 → 재설치" 흐름), "삭제" 메뉴만 항상 전체 삭제라
/// 비대칭이었던 걸 맞춘다.
///
/// 로컬 루트 오버라이드가 설정된 항목은 거부한다 — 그 경로는 사용자가 직접 지정한
/// 임의 폴더라, PengPort 가 자동으로 지워도 되는 데이터인지 알 수 없다.
#[tauri::command]
pub async fn library_delete_installed_data(
    id: String,
    groups: Option<Vec<String>>,
) -> Result<(), String> {
    let mut store = load_library_store().await?;
    let entry = store.get(&id).ok_or_else(|| format!("알 수 없는 레시피: {id}"))?;
    if entry.local_root_override.is_some() {
        return Err(
            "로컬 경로 오버라이드가 설정된 항목입니다 — 사용자가 직접 지정한 폴더라 자동 삭제하지 않습니다. 필요하면 직접 삭제하세요."
                .to_string(),
        );
    }
    let recipe = entry.recipe.clone();

    match groups {
        None => {
            // ThirdPartyAppLaunch(예: Prism 기반 Minecraft 인스턴스)는 third-party
            // app 자체를 찾아야 그 인스턴스 폴더 위치도 알 수 있다 — Prism이 삭제됐거나
            // 옮겨져 못 찾으면 어디를 지워야 할지 PengPort가 알 방법이 없다. 이걸 못
            // 찾는다고 삭제 전체(마커 정리까지)를 막으면 "라이브러리에서 제거"가 영영
            // 안 되는 궁지에 빠진다 — "로컬 경로 오버라이드"와 같은 성격(PengPort가
            // 손댈 수 없는 대상)이라 조용히 skip 하고, PengPort 자신이 아는 마커
            // 폴더(third-party app 위치와 무관)는 그대로 정리한다.
            let target_root = resolve_target_root(&recipe.id, &recipe.launch).ok();
            let markers_root = super::paths::app_root(&recipe.id)
                .ok_or_else(|| "app_root 미정 (%LOCALAPPDATA% 환경변수 없음)".to_string())?;

            tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
                if let Some(target_root) = &target_root {
                    if target_root.exists() {
                        terminate_processes_under(target_root)?;
                        std::fs::remove_dir_all(target_root)
                            .map_err(|e| format!("설치 데이터 삭제 실패 ({}): {e}", target_root.display()))?;
                    }
                }
                // SpawnProcess 는 target_root == markers_root(같은 apps/<id>/) 라 위에서 이미 지워짐.
                // ThirdPartyAppLaunch 는 markers_root 가 별도 위치(apps/<id>/.pengport-markers 만
                // 들어있는 작은 폴더)라 따로 지워야 함. target_root 를 못 찾은 경우(위 주석)에도
                // 이 마커 폴더는 PengPort 자신의 위치라 항상 정리 가능.
                if target_root.as_deref() != Some(markers_root.as_path()) && markers_root.exists() {
                    std::fs::remove_dir_all(&markers_root)
                        .map_err(|e| format!("설치 마커 삭제 실패 ({}): {e}", markers_root.display()))?;
                }
                Ok(())
            })
            .await
            .map_err(|e| format!("blocking task panic: {e}"))??;

            store.set_selected_optional_groups(&recipe.id, None);
        }
        Some(group_ids) => {
            let group_set: HashSet<String> = group_ids.into_iter().collect();
            let root = resolve_target_root(&recipe.id, &recipe.launch)?;
            let markers_dir = super::paths::app_root(&recipe.id)
                .ok_or_else(|| "app_root 미정 (%LOCALAPPDATA% 환경변수 없음)".to_string())?
                .join(".pengport-markers");

            let archives_to_clear: Vec<ArchiveExtraction> = recipe
                .archives
                .iter()
                .filter(|a| a.optional_group.as_ref().is_some_and(|g| group_set.contains(g)))
                .cloned()
                .collect();
            let files_to_clear: Vec<RecipeFile> = recipe
                .files
                .iter()
                .filter(|f| f.optional_group.as_ref().is_some_and(|g| group_set.contains(g)))
                .cloned()
                .collect();
            let recipe_for_ancestors = recipe.clone();

            tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
                terminate_processes_under(&root)?;
                for archive in &archives_to_clear {
                    // 매니페스트 있으면 이 그룹이 실제로 쓴 파일만 정밀 삭제(같은 폴더에
                    // 다른 압축이 넣어둔 콘텐츠는 보존), 없으면 통째 삭제 + 조상 마커
                    // 무효화로 폴백(`remove_grouped_archive_content` 문서 참고).
                    remove_grouped_archive_content(&recipe_for_ancestors, &root, &markers_dir, archive)?;
                }
                for file in &files_to_clear {
                    if file.override_content.is_some() {
                        remove_marker(&markers_dir, &file_override_hash(file)?)?;
                    }
                }
                Ok(())
            })
            .await
            .map_err(|e| format!("blocking task panic: {e}"))??;

            // 이미 확정된 선택이 있었으면 지운 그룹들을 거기서도 빼서 반영(다음 설치 때
            // "아직 켜져 있다"고 오판해 재다운로드를 건너뛰지 않게).
            if let Some(existing) = store.get(&recipe.id).and_then(|e| e.selected_optional_groups.clone()) {
                let updated: HashSet<String> = existing.difference(&group_set).cloned().collect();
                store.set_selected_optional_groups(&recipe.id, Some(updated));
            }
        }
    }

    store.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn library_remove(id: String) -> Result<bool, String> {
    let mut store = load_library_store().await?;
    tauri::async_runtime::spawn_blocking(move || {
        let removed = store.remove(&id);
        store.save().map_err(|e| e.to_string())?;
        Ok(removed)
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// 라이브러리 카드 순서(드래그로 재배치) 저장 — `ids`는 프론트가 화면에 보여줄
/// 순서 그대로 넘긴 전체 id 목록. `LibraryStore::reorder` 참고(빠진/모르는 id 는
/// 데이터 손실 없이 안전하게 처리).
#[tauri::command]
pub async fn library_reorder(ids: Vec<String>) -> Result<(), String> {
    let mut store = load_library_store().await?;
    tauri::async_runtime::spawn_blocking(move || {
        store.reorder(&ids);
        store.save().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

// ---------------------------------------------------------------------------
// library_install — 설치/업데이트(같은 동작). library_launch — 레시피 실행(설치 안
// 함, 설치 안 돼 있으면 명확한 에러).
//
// 신뢰는 "라이브러리에 있다" = 이미 임포트 시점에 확인됐다는 뜻이라, 실행은 항상
// 바로 진행된다 — 대신 아티팩트(다운로드물)는 매번 자동으로 검증된다.
//
// "설치"와 "업데이트"가 버튼은 둘이지만 커맨드는 하나인 이유: 마커가 항목 존재 여부가
// 아니라 **항목 내용의 해시**라서, 처음 설치든("전부 마커 없음 → 전부 적용") 나중에
// 레시피가 바뀐 뒤 다시 누르든("바뀐 항목만 마커가 안 맞음 → 그것만 재적용") 똑같이
// "지금 레시피와 실제 설치 상태를 맞춘다"는 동작이다.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallOutcome {
    /// `updated`=이번에 실제로 적용된 항목 수(0이면 "이미 최신 상태"), `total`=전체.
    Completed { updated: usize, total: usize },
    /// 로컬 루트 오버라이드 사용 중 — 사용자가 이미 설치된 폴더를 직접 지정했으므로
    /// PengPort 가 덮어쓸 설치 단계 자체가 없음.
    UsingLocalOverride,
    /// third-party app(예: Prism)이 시스템에 없어 설치를 진행할 수 없음 — frontend 가
    /// 설치 dialog 표시 후 재시도해야 함.
    ThirdPartyAppMissing { app_id: String },
    /// 레시피에 [`Recipe::optional_groups`]가 있는데 아직 한 번도 선택을 확인 안 함 —
    /// frontend 가 선택 다이얼로그 표시 후 [`library_set_selected_optional_groups`]로
    /// 확정하고 재시도해야 함(third-party 앱 없음과 같은 재시도 패턴).
    NeedsOptionalGroupSelection,
    /// 사용자가 [`library_cancel_install`]로 도중에 멈춤 — 에러가 아니라 정상적인
    /// 사용자 의사결정이라 별도 kind 로 구분(프론트가 빨간 에러 토스트 대신 조용한
    /// 안내만 보여줄 수 있게). 이미 적용된 항목까지 되돌리진 않는다 — 다음 설치 때
    /// 마커 기준으로 이어서 진행(크래시 복구와 같은 방식).
    Cancelled,
    /// `Literal` override 파일 중 선언값은 바뀌었는데, 디스크의 실제 내용이 PengPort가
    /// 마지막으로 쓴 것과 달라진(=사용자가 그 사이 직접 건드린) 항목이 있음 — 그냥
    /// 덮어쓰면 그 변경이 사라진다. frontend 가 3선택지 다이얼로그를 띄우고
    /// [`library_resolve_override_conflicts`]로 각 파일을 해결한 뒤 재시도해야 함.
    HasOverrideConflicts { conflicts: Vec<OverrideConflict> },
    /// 압축 해제 대상(전체 허용 + `ask_on_conflict` 폴더) 안에 이름은 같고 내용은
    /// 다른 파일이 이미 있음 — frontend 가 충돌 다이얼로그를 띄우고
    /// [`library_resolve_archive_conflicts`]로 해결한 뒤 재시도해야 함.
    HasArchiveConflicts { archives: Vec<ArchiveConflictGroup> },
}

/// [`InstallOutcome::HasArchiveConflicts`] 항목 하나 — 압축 하나에서 발견된 충돌
/// 전부. `archive_hash`는 [`library_resolve_archive_conflicts`]가 어느 압축인지
/// 식별하는 키([`archive_content_hash`]와 동일).
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveConflictGroup {
    pub archive_hash: String,
    pub url: String,
    pub conflicts: Vec<String>,
}

/// [`InstallOutcome::HasOverrideConflicts`] 항목 하나 — 드리프트가 감지된 파일의
/// 경로. v1은 경로만 보여준다(내용 미리보기/diff는 범위 밖).
#[derive(Debug, Clone, Serialize)]
pub struct OverrideConflict {
    pub path: String,
}

#[tauri::command]
pub async fn library_install(id: String, app: tauri::AppHandle) -> Result<InstallOutcome, String> {
    pengport_shared::validate_service_id(&id).map_err(|e| format!("레시피 id 형식 오류 ({id:?}): {e}"))?;

    let store = load_library_store().await?;
    let entry = store.get(&id).ok_or_else(|| format!("알 수 없는 레시피: {id}"))?;
    if entry.local_root_override.is_some() {
        return Ok(InstallOutcome::UsingLocalOverride);
    }
    let recipe = entry.recipe.clone();
    let selection = entry.selected_optional_groups.clone();

    let ctx = ActionContext {
        allow_http: allow_http(),
    };
    validate_recipe(&recipe, &ctx).map_err(|e| e.to_string())?;

    if !recipe.optional_groups.is_empty() && selection.is_none() {
        return Ok(InstallOutcome::NeedsOptionalGroupSelection);
    }

    for app_id in referenced_third_party_app_ids(&recipe) {
        if check_third_party_app_available(&app_id).is_err() {
            return Ok(InstallOutcome::ThirdPartyAppMissing { app_id });
        }
    }

    let selected = selection.clone().unwrap_or_default();
    {
        let recipe = recipe.clone();
        let selected = selected.clone();
        let conflicts = tauri::async_runtime::spawn_blocking(move || -> Result<Vec<OverrideConflict>, String> {
            let root = resolve_target_root(&recipe.id, &recipe.launch)?;
            let markers_dir = super::paths::app_root(&recipe.id)
                .ok_or_else(|| "app_root 미정 (%LOCALAPPDATA% 환경변수 없음)".to_string())?
                .join(".pengport-markers");
            detect_override_conflicts(&recipe, &selected, &root, &markers_dir)
        })
        .await
        .map_err(|e| format!("blocking task panic: {e}"))??;
        if !conflicts.is_empty() {
            return Ok(InstallOutcome::HasOverrideConflicts { conflicts });
        }
    }

    match reconcile_install(&recipe, &selection.unwrap_or_default(), &app).await {
        // 정확히 같은 문자열이 아니라 포함 여부로 본다 — 취소 시그널이 7z 추출
        // 경로처럼 `format!("...: {e}")`로 한 번 더 감싸이는 지점을 지날 수 있어서
        // (zip 경로는 안 감싸이지만 7z 경로는 감싸임), 어느 경로로 와도 잡아내려면
        // 문자열 포함 검사가 더 안전하다.
        Err(e) if e.contains(INSTALL_CANCELLED_SENTINEL) => Ok(InstallOutcome::Cancelled),
        other => other,
    }
}

/// [`InstallOutcome::HasArchiveConflicts`]에 담겨온 압축 안 엔트리 하나를 어떻게
/// 처리할지 — frontend 의 충돌 다이얼로그가 고른 값. [`Serialize`]도 필요한 이유는
/// [`OverrideConflictResolution`]과 달리 이 값이 `.pengport-tmp-pending/*.resolutions.json`에
/// 그대로 저장됐다가 재시도 때 다시 읽히기 때문(프론트→백엔드 1회성 입력이 아니라
/// 디스크에 영속되는 값).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ArchiveEntryResolution {
    /// 기존 파일을 압축 안 내용으로 덮어씀.
    Overwrite { path: String },
    /// 이 엔트리는 추출하지 않고 기존 파일을 그대로 둠.
    Skip { path: String },
    /// 기존 파일은 그대로 두고, 압축 안 내용은 [`unique_fs_path`]로 새 이름을 받아
    /// 같은 폴더에 따로 씀("이름 (2).ext"). "전체 허용" 폴더에서만 의미 있다 —
    /// 화이트리스트 강제 폴더였다면 이 새 이름은 레시피가 모르는 파일이라 다음
    /// 정리 때 바로 지워진다(그래서 이 기능 자체가 전체 허용 폴더로 범위 한정됨).
    Rename { path: String },
}

/// [`InstallOutcome::HasOverrideConflicts`]에 담겨온 파일 하나를 어떻게 처리할지 —
/// frontend 의 3선택지 다이얼로그가 고른 값 그대로.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum OverrideConflictResolution {
    /// 새 레시피 내용으로 덮어씀 — 여기선 아무것도 안 함(다음 `library_install`
    /// 재시도가 declined 마커도 없고 지문도 안 맞을 리 없으니 그냥 정상 적용됨).
    /// frontend는 다른 액션과 형태를 맞추려고 `path`를 같이 보내지만(균일한 배열),
    /// 여기선 실제로 쓰이지 않아 필드 자체를 안 둠(serde가 알 수 없는 JSON 필드는
    /// 조용히 무시).
    Overwrite,
    /// 이번엔 건너뜀 — 지금 선언값(해시) 기준으로 declined 마커를 남겨서, 레시피가
    /// 또 바뀌기 전까진 다시 안 물어봄.
    Skip { path: String },
    /// 디스크의 지금 내용을 이 레시피의 새 선언값으로 채택(로컬 사본만 갱신 —
    /// 외부에 공유되는 원본 레시피 소스에는 반영 안 됨).
    AdoptDisk { path: String },
}

/// [`OverrideConflictResolution::AdoptDisk`] — 디스크에서 읽은 원본 바이트를
/// `FileContent`로 감싼다. `FileContent`가 텍스트 전용(2026-08 보안 강화로 바이너리
/// 리터럴 제거 — [`FileContent`] 문서 참고)이라, UTF-8이 아닌 파일은 애초에 리터럴
/// override로 담을 수 없다 — 그런 파일은 `ArchiveExtraction`으로만 반영 가능하다는
/// 뜻이라 명확한 에러로 알린다.
fn adopt_disk_content(disk_bytes: Vec<u8>) -> Result<FileContent, String> {
    String::from_utf8(disk_bytes)
        .map(|text| FileContent::Text { content: text })
        .map_err(|_| {
            "이 파일은 텍스트가 아니라 리터럴 override로 담을 수 없습니다 \
             (바이너리 자산은 압축 다운로드로만 설치 가능)"
                .to_string()
        })
}

/// [`InstallOutcome::HasOverrideConflicts`] 확인 후 frontend 가 사용자의 선택을
/// 반영하는 커맨드 — 처리 후 [`library_install`]을 다시 호출하면 이어서 진행된다
/// (third-party 앱 설치/선택 그룹 확정과 같은 "해결 후 재시도" 패턴).
#[tauri::command]
pub async fn library_resolve_override_conflicts(
    id: String,
    resolutions: Vec<OverrideConflictResolution>,
) -> Result<(), String> {
    let mut store = load_library_store().await?;
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let entry = store.get(&id).ok_or_else(|| format!("알 수 없는 레시피: {id}"))?;
        let mut recipe = entry.recipe.clone();
        let root = resolve_target_root(&recipe.id, &recipe.launch)?;
        let markers_dir = super::paths::app_root(&recipe.id)
            .ok_or_else(|| "app_root 미정 (%LOCALAPPDATA% 환경변수 없음)".to_string())?
            .join(".pengport-markers");

        let mut changed = false;
        for resolution in resolutions {
            match resolution {
                OverrideConflictResolution::Overwrite => {}
                OverrideConflictResolution::Skip { path } => {
                    let file = recipe
                        .files
                        .iter()
                        .find(|f| f.path == path)
                        .ok_or_else(|| format!("레시피에 없는 파일 경로: {path}"))?;
                    let hash = file_override_hash(file)?;
                    write_declined_marker(&markers_dir, &hash)?;
                }
                OverrideConflictResolution::AdoptDisk { path } => {
                    let disk_bytes = std::fs::read(root.join(&path))
                        .map_err(|e| format!("파일 읽기 실패 ({path}): {e}"))?;
                    let file = recipe
                        .files
                        .iter_mut()
                        .find(|f| f.path == path)
                        .ok_or_else(|| format!("레시피에 없는 파일 경로: {path}"))?;
                    let content = adopt_disk_content(disk_bytes)?;
                    file.override_content = Some(OverrideContent::Literal { content });
                    changed = true;
                }
            }
        }

        if changed {
            store.upsert(recipe);
        }
        store.save().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// [`InstallOutcome::HasArchiveConflicts`] 확인 후 frontend 가 사용자의 선택을 그
/// 압축의 대기 폴더에 저장만 한다(실제 적용은 다음 [`library_install`] 재시도 때
/// `execute_archive`가 읽어서 반영 — 그때 보존해둔 다운로드도 같이 재사용되므로
/// 재다운로드가 없다).
#[tauri::command]
pub async fn library_resolve_archive_conflicts(
    id: String,
    archive_hash: String,
    resolutions: Vec<ArchiveEntryResolution>,
) -> Result<(), String> {
    pengport_shared::validate_service_id(&id).map_err(|e| format!("레시피 id 형식 오류 ({id:?}): {e}"))?;
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let pending_dir = archive_pending_conflict_dir(&id)?;
        write_pending_resolutions(&pending_dir, &archive_hash, &resolutions)
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// [`library_install`]과 같은 원장(마커)을 조회하되 **아무것도 실행하지 않는** 조회
/// 전용 버전 — 카드에 "미설치"/"업데이트 필요" 뱃지를 보여주기 위함(부작용 없음, 네트워크
/// 요청도 안 함).
///
/// pending 항목이 하나라도 있고, 마커 폴더에 다른 마커가 이미 있으면(=전에 뭔가는
/// 설치된 적 있다는 증거) "업데이트 필요", 마커가 아예 없으면 "미설치"로 본다 — 지금
/// 실제 파일 내용을 레시피 선언값과 비교하지 않는다(모듈 설명 참고 — 런타임에 생기는
/// 정상적인 변화까지 업데이트 필요로 오판하는 걸 막기 위함).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallStatus {
    /// 모든 항목이 마커상 이미 적용된 상태.
    UpToDate,
    /// 마커가 하나도 없음 — 한 번도 설치 안 함.
    NotInstalled,
    /// 일부 항목의 마커가 없지만, 다른 항목은 마커가 있음 — 레시피가 바뀌었거나 처음
    /// 설치 이후 새 항목이 추가된 경우.
    UpdateAvailable { pending: usize, total: usize },
    /// 로컬 루트 오버라이드 사용 중 — 설치 상태 개념 자체가 없음.
    UsingLocalOverride,
    /// [`InstallOutcome::NeedsOptionalGroupSelection`]과 같은 이유 — 아직 선택을 확인
    /// 안 해서 설치 상태 자체를 계산할 수 없음.
    NeedsOptionalGroupSelection,
}

#[tauri::command]
pub async fn library_install_status(id: String) -> Result<InstallStatus, String> {
    let store = load_library_store().await?;
    let entry = store
        .get(&id)
        .ok_or_else(|| format!("알 수 없는 레시피: {id}"))?;
    if entry.local_root_override.is_some() {
        return Ok(InstallStatus::UsingLocalOverride);
    }
    let recipe = entry.recipe.clone();
    if !recipe.optional_groups.is_empty() && entry.selected_optional_groups.is_none() {
        return Ok(InstallStatus::NeedsOptionalGroupSelection);
    }
    let selected = entry.selected_optional_groups.clone().unwrap_or_default();

    let markers_dir = super::paths::app_root(&recipe.id)
        .ok_or_else(|| "app_root 미정 (%LOCALAPPDATA% 환경변수 없음)".to_string())?
        .join(".pengport-markers");

    tauri::async_runtime::spawn_blocking(move || -> Result<InstallStatus, String> {
        let effective = effective_files(&recipe, &selected);
        let mut total = effective.iter().filter(|f| f.override_content.is_some()).count();
        let mut pending = 0usize;

        for archive in &recipe.archives {
            let in_scope = match &archive.optional_group {
                None => true,
                Some(group) => selected.contains(group),
            };
            let hash = archive_content_hash(archive)?;
            if in_scope {
                total += 1;
                if !marker_exists(&markers_dir, &hash) {
                    pending += 1;
                }
            } else if marker_exists(&markers_dir, &hash) {
                // 그룹 해제됐지만 아직 콘텐츠가 안 지워짐 — 정리 작업 대기 중.
                pending += 1;
            }
        }
        for file in effective.iter().filter(|f| f.override_content.is_some()) {
            if !is_resolved(&markers_dir, &file_override_hash(file)?) {
                pending += 1;
            }
        }

        Ok(if pending == 0 {
            InstallStatus::UpToDate
        } else if has_any_marker(&markers_dir) {
            InstallStatus::UpdateAvailable { pending, total }
        } else {
            InstallStatus::NotInstalled
        })
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// "업데이트 필요"가 왜 뜨는지 — **어느 항목이** 아직 반영된 적 없는지 보여준다.
/// `library_install_status`는 카드마다 항상 조회되는 가벼운 커맨드라 요약(개수)만
/// 주지만, 이건 사용자가 뱃지를 눌렀을 때만 온디맨드로 호출된다.
///
/// 항목 내용의 "어느 부분이 다른지"까지는 보여주지 않는다 — 설치 이후 앱 사용으로
/// 생기는 정상적인 변화(런타임 캐시/설정)와 진짜 레시피 변경을 실제 파일 비교로는
/// 구분할 수 없다는 게 이 세션에서 확인된 근본 이유(모듈 설명 참고). "아직 반영된
/// 적 없다"는 원장 사실만 정직하게 보여준다.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallDiagnostic {
    /// 압축 다운로드+해제(+화이트리스트 정리)가 아직 반영된 적 없음(마커 기준).
    /// `missing_paths`는 이 압축의 목적지 아래 선언된 파일 중 지금 디스크에 실제로
    /// 없는 것만 골라 보여준다 — **마커는 없는데 파일은 이미 다 있는 경우**(원장
    /// 기록과 실제 디스크 상태가 어긋난 경우)와 **진짜로 파일이 없는 경우**를 사용자가
    /// 직접 구분할 수 있게 하기 위함. 레시피는 파일별로 "이건 어느 압축 소유"를
    /// 선언하지 않으므로(`extract_to` 접두사로만 근사) — 같은 목적지를 공유하는 다른
    /// 압축의 파일까지 같이 잡힐 수 있다는 한계는 있음.
    ArchivePending { url: String, missing_paths: Vec<String> },
    /// 파일 오버라이드가 아직 반영된 적 없음.
    FilePending { path: String },
    /// [`InstallOutcome::NeedsOptionalGroupSelection`]과 같은 이유 — 아직 선택을 확인
    /// 안 해서 개별 항목 pending 여부 자체를 계산할 수 없다(계산하면 방금 생긴 옵션이
    /// "범위 밖"으로 오판돼 잘못된 진단이 됨). 이게 뜨면 다른 진단 없이 이것만 반환한다.
    NeedsOptionalGroupSelection,
}

/// [`missing_declared_files`]가 쓰는 "부모 디렉토리(대상 루트 기준 **상대** 경로) →
/// 그 안에 실제로 있는 파일명들" 캐시. 키를 절대경로가 아니라 상대경로로 두는 이유는
/// 조회 쪽(`missing_declared_files`)이 `RecipeFile.path`에서 부모를 뽑을 때 `root`와
/// 합치는 `PathBuf` 할당 없이(`Path::parent()`는 빌린 슬라이스라 공짜) 바로 조회할 수
/// 있게 하기 위함 — 파일이 수천 개면 이 할당 하나하나가 실측상 유의미하게 쌓인다.
/// `effective` 전체를 대상으로 [`build_dir_listing_cache`]가 한 번만 만들고, 압축이
/// 여러 개 pending 이어도(특히 서로 범위가 겹치는 경우) 이 함수 호출 전체에서
/// 재사용해서 같은 디렉토리를 두 번 안 읽는다. `None`은 그 디렉토리 자체가 없거나
/// 못 읽음(전부 없는 것으로 취급).
type DirListingCache = HashMap<PathBuf, Option<HashSet<std::ffi::OsString>>>;

/// `effective`에 선언된 모든 파일을 부모 디렉토리별로 묶어 `read_dir`을 디렉토리당
/// 딱 한 번만 불러 캐시를 만든다. `saves`/`screenshots`처럼 선언된 파일이 하나도 없는
/// 디렉토리는 애초에 이 맵에 안 들어가므로 안 건드린다(그 안에 뭐가 많든 무관).
fn build_dir_listing_cache(root: &Path, effective: &[&RecipeFile]) -> DirListingCache {
    let mut parents: HashSet<PathBuf> = HashSet::new();
    for f in effective {
        let parent = Path::new(&f.path).parent().unwrap_or_else(|| Path::new(""));
        parents.insert(parent.to_path_buf());
    }
    let parents: Vec<PathBuf> = parents.into_iter().collect();

    // `read_dir` 하나하나는 CPU 가 아니라 디스크 I/O 대기가 대부분이라, 디렉토리
    // 개수가 수백 개인 콘텐츠 팩에서는 순차 호출보다 여러 스레드로 동시에 부르는 쪽이
    // 벽시계 시간을 크게 줄인다(실측 근거로 도입 — 콘텐츠 팩 하나가 디렉토리 700개+).
    let worker_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(parents.len().max(1));
    let next_index = std::sync::atomic::AtomicUsize::new(0);
    let results = Mutex::new(Vec::with_capacity(parents.len()));

    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| loop {
                let i = next_index.fetch_add(1, Ordering::Relaxed);
                let Some(parent) = parents.get(i) else { break };
                let names: Option<HashSet<std::ffi::OsString>> = std::fs::read_dir(root.join(parent))
                    .ok()
                    .map(|entries| entries.filter_map(|e| e.ok()).map(|e| e.file_name()).collect());
                results.lock().unwrap().push((parent.clone(), names));
            });
        }
    });

    results.into_inner().unwrap().into_iter().collect()
}

/// `extract_to`(빈 문자열이면 전체) 아래에 해당하는 `Recipe.files` 선언 중, 지금
/// 디스크에 실제로 없는 것만 상대경로로 반환. 같은 `extract_to`를 공유하는 압축이
/// 여럿이면 레시피 데이터만으로는 "이 파일이 정확히 어느 압축 소유인지" 구분할 수
/// 없어 근사치일 뿐이지만, "마커만 없고 파일은 이미 있다"를 가려내는 덴 충분하다.
///
/// 파일 하나하나 `exists()`(개별 syscall)를 부르는 대신 [`DirListingCache`]를 조회만
/// 한다 — 실제 디스크 접근은 [`build_dir_listing_cache`]가 호출부에서 한 번만 미리
/// 해둔다(콘텐츠 팩처럼 파일이 수천 개인 압축에서 수천 번의 개별 stat 대신 **부모
/// 디렉토리 개수만큼만** 시스템 콜, 여러 압축이 겹치는 범위를 pending 으로 봐도 중복
/// 없음 — 실측: 콘텐츠 팩 하나가 파일 7000개+ 인데 부모 디렉토리는 수백 개뿐). 캐시
/// 키가 상대경로라 여기서도 `root`를 합치는 `PathBuf` 할당이 후보 파일마다 필요 없다.
fn missing_declared_files(extract_to: &str, effective: &[&RecipeFile], cache: &DirListingCache) -> Vec<String> {
    let prefix = if extract_to.is_empty() { None } else { Some(format!("{extract_to}/")) };
    effective
        .iter()
        .filter(|f| prefix.as_deref().is_none_or(|p| f.path.starts_with(p)))
        .filter(|f| {
            let path = Path::new(&f.path);
            let parent = path.parent().unwrap_or_else(|| Path::new(""));
            let exists = cache
                .get(parent)
                .and_then(|names| names.as_ref())
                .is_some_and(|names| path.file_name().is_some_and(|n| names.contains(n)));
            !exists
        })
        .map(|f| f.path.clone())
        .collect()
}

#[tauri::command]
pub async fn library_install_diagnostics(id: String) -> Result<Vec<InstallDiagnostic>, String> {
    let store = load_library_store().await?;
    let entry = store
        .get(&id)
        .ok_or_else(|| format!("알 수 없는 레시피: {id}"))?;
    if entry.local_root_override.is_some() {
        return Ok(Vec::new());
    }
    let recipe = entry.recipe.clone();
    // `library_install`(실제 설치)과 같은 조건으로 같은 결론을 내야 한다 — 여기서
    // `selection.unwrap_or_default()`로 그냥 넘어가면 "아직 한 번도 선택 확인 안 함"과
    // "확인했고 전부 해제함"을 구분 못 해, 방금 옵션이 생긴 압축이 실제로는 선택 창이
    // 뜰 텐데도 "범위 밖이라 마커만 없다"는 잘못된 진단으로 보이는 사고가 난다.
    if !recipe.optional_groups.is_empty() && entry.selected_optional_groups.is_none() {
        return Ok(vec![InstallDiagnostic::NeedsOptionalGroupSelection]);
    }
    let selected = entry.selected_optional_groups.clone().unwrap_or_default();

    let markers_dir = super::paths::app_root(&recipe.id)
        .ok_or_else(|| "app_root 미정 (%LOCALAPPDATA% 환경변수 없음)".to_string())?
        .join(".pengport-markers");

    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<InstallDiagnostic>, String> {
        let root = resolve_target_root(&recipe.id, &recipe.launch)?;
        let effective = effective_files(&recipe, &selected);

        // 어떤 압축이 pending 인지 먼저 다 판정(마커 조회만 — 디스크 스캔 없음).
        // pending 압축이 하나도 없으면(파일 오버라이드만 pending) 아래 디렉토리 캐시
        // 자체를 안 만든다 — 디스크 접근을 진짜 필요할 때만 하기 위함.
        let mut pending_archives = Vec::new();
        for archive in &recipe.archives {
            let in_scope = match &archive.optional_group {
                None => true,
                Some(group) => selected.contains(group),
            };
            let hash = archive_content_hash(archive)?;
            let is_pending = if in_scope {
                !marker_exists(&markers_dir, &hash)
            } else {
                marker_exists(&markers_dir, &hash)
            };
            if is_pending {
                pending_archives.push(archive);
            }
        }

        let mut out = Vec::new();
        if !pending_archives.is_empty() {
            // pending 압축 전체가 공유하는 캐시 — 범위가 겹치는 압축이 여럿이어도
            // (예: 하나가 다른 하나의 상위 폴더를 통째로 선언) 같은 디렉토리를 두 번
            // 안 읽는다.
            let cache = build_dir_listing_cache(&root, &effective);
            // 압축 여러 개가 같은 `extract_to`를 선언하면(예: 같은 폴더에 raw_filename
            // 단일 파일을 여러 개 두는 경우) "이 범위 밑에 뭐가 없나" 결과가 완전히
            // 똑같으므로, extract_to 별로 한 번만 계산해서 재사용한다.
            let mut missing_by_extract_to: HashMap<&str, Vec<String>> = HashMap::new();
            for archive in &pending_archives {
                missing_by_extract_to
                    .entry(archive.extract_to.as_str())
                    .or_insert_with(|| missing_declared_files(&archive.extract_to, &effective, &cache));
            }
            for archive in pending_archives {
                let missing_paths = missing_by_extract_to[archive.extract_to.as_str()].clone();
                out.push(InstallDiagnostic::ArchivePending { url: archive.url.clone(), missing_paths });
            }
        }
        for file in effective
            .iter()
            .copied()
            .filter(|f| f.override_content.is_some())
        {
            if !is_resolved(&markers_dir, &file_override_hash(file)?) {
                out.push(InstallDiagnostic::FilePending { path: file.path.clone() });
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchOutcome {
    Launched,
    /// `LaunchAction::ThirdPartyAppLaunch`의 대상 앱이 시스템에 없음 — frontend 가 설치
    /// dialog 표시 후 재시도해야 함.
    ThirdPartyAppMissing { app_id: String },
}

#[tauri::command]
pub async fn library_launch(id: String, app: tauri::AppHandle) -> Result<LaunchOutcome, String> {
    pengport_shared::validate_service_id(&id).map_err(|e| format!("레시피 id 형식 오류 ({id:?}): {e}"))?;

    let store = load_library_store().await?;
    let entry = store.get(&id).ok_or_else(|| format!("알 수 없는 레시피: {id}"))?;
    let recipe = entry.recipe.clone();
    let root_override = entry.local_root_override.clone();

    let ctx = ActionContext {
        allow_http: allow_http(),
    };
    validate_recipe(&recipe, &ctx).map_err(|e| e.to_string())?;

    match &recipe.launch {
        LaunchAction::SpawnProcess {
            entry_point,
            entry_args,
        } => {
            let root_dir = match root_override {
                Some(root) => root,
                None => resolve_target_root(&recipe.id, &recipe.launch)?,
            };
            spawn_local_process(&root_dir, entry_point, entry_args)?;
            Ok(LaunchOutcome::Launched)
        }
        LaunchAction::ThirdPartyAppLaunch { app_id } => {
            if check_third_party_app_available(app_id).is_err() {
                return Ok(LaunchOutcome::ThirdPartyAppMissing {
                    app_id: app_id.clone(),
                });
            }
            super::third_party_runtime::spawn_third_party_app(&app, app_id, &recipe.id)?;
            Ok(LaunchOutcome::Launched)
        }
    }
}

/// third-party app descriptor 로컬 저장소 위치 — 레시피(`library_store_path`)와 같은
/// 패턴. PengPort 는 태생적으로 아는 third-party app 이 하나도 없다(빈 파일/파일 없음 =
/// 빈 목록) — `.pengz` 임포트(`import::commit_file`)가 레시피와 함께 descriptor 도 여기 반영한다.
fn third_party_apps_store_path() -> Result<PathBuf, String> {
    super::paths::app_data_root()
        .map(|d| d.join("third_party_apps.json"))
        .ok_or_else(|| "%APPDATA% 미정 (third_party_apps.json 위치 결정 불가)".to_string())
}

/// 지금 PengPort 가 아는 third-party app descriptor 전체 — 데이터 루트 해석
/// (`third_party_app_instance_dir`/`check_third_party_app_available`)이 조회하는 목록.
/// 새 third-party 플랫폼 지원의 유일한 추가 경로는 이제 코드/리소스 변경이 아니라 링크
/// 임포트(또는 개발 중 로컬 파일 직접 편집) — 경로 해석 알고리즘 자체는 여전히
/// `resolve_third_party_app`(범용) 하나. `docs/design/THIRD_PARTY_PLATFORM_MODEL.md` 참고.
///
/// 파싱 실패/version 불일치는 [`LibraryStore`]와 동일하게 에러로 전파한다(조용히 빈
/// 목록으로 폴백하지 않음 — 손상된 파일을 "third-party app 이 하나도 없음"으로 오인해
/// 사용자가 애써 등록한 descriptor 를 못 찾는 걸 숨기지 않기 위함).
pub(super) fn known_third_party_apps() -> Result<Vec<pengport_shared::actions::ThirdPartyAppDescriptor>, String> {
    let path = third_party_apps_store_path()?;
    pengport_shared::library::ThirdPartyAppStore::load(path)
        .map(|store| store.list().to_vec())
        .map_err(|e| e.to_string())
}

/// 레시피 편집 화면(`RecipeEditDialog.tsx`)의 "대상 서드파티 앱" 드롭다운이 쓰는
/// id 목록 — 자유 텍스트 입력 대신 등록된 descriptor 중에서만 고를 수 있게 한다.
#[tauri::command]
pub fn list_third_party_app_ids() -> Result<Vec<String>, String> {
    Ok(known_third_party_apps()?.into_iter().map(|d| d.id).collect())
}

/// 설정 화면(`ThirdPartyApps.tsx`)이 카드 목록을 렌더링할 때 쓰는 id+표시 이름 요약 —
/// `id`만 필요한 [`list_third_party_app_ids`]와 달리 사람이 읽을 라벨까지 필요해서
/// 별도 반환 타입을 둔다. `supports_download`는 프론트가 "자동 다운로드" 버튼을 보일지
/// 결정 — 옛 `AUTO_DOWNLOAD` 하드코딩 맵(app_id 별로 프론트 코드에 함수를 등록해야
/// 했음)을 대체한다: 이제 descriptor 에 `download_strategy`가 있는지만 보면 된다.
#[derive(Debug, Clone, Serialize)]
pub struct ThirdPartyAppSummary {
    pub id: String,
    pub label: String,
    pub supports_download: bool,
}

#[tauri::command]
pub fn list_third_party_apps() -> Result<Vec<ThirdPartyAppSummary>, String> {
    Ok(known_third_party_apps()?
        .into_iter()
        .map(|d| ThirdPartyAppSummary {
            label: d.label.clone().unwrap_or_else(|| d.id.clone()),
            supports_download: d.download_strategy.is_some(),
            id: d.id,
        })
        .collect())
}

/// 등록된 third-party app descriptor 전체(모든 필드) — 설정 화면의 편집 다이얼로그가
/// 기존 값을 채워 넣는 데 쓴다. `list_third_party_apps`(요약)와 달리 편집에 필요한
/// 모든 필드를 그대로 반환.
#[tauri::command]
pub fn list_third_party_app_descriptors(
) -> Result<Vec<pengport_shared::actions::ThirdPartyAppDescriptor>, String> {
    known_third_party_apps()
}

/// 서드파티 앱 descriptor 직접 추가/갱신("설정 화면에서 직접 등록" 경로 — 링크
/// 임포트가 아닌 경우, `library_upsert`의 대응). `id`는 다운로드 시 bundled root
/// (`%LOCALAPPDATA%\PengPort\<id>\`)의 경로 컴포넌트로도 쓰이므로 레시피 id 와
/// 동일하게 검증한다.
#[tauri::command]
pub async fn third_party_app_upsert(descriptor: pengport_shared::actions::ThirdPartyAppDescriptor) -> Result<(), String> {
    pengport_shared::validate_service_id(&descriptor.id)
        .map_err(|e| format!("서드파티 앱 id 형식 오류 ({:?}): {e}", descriptor.id))?;

    let mut store = load_third_party_app_store().await?;
    tauri::async_runtime::spawn_blocking(move || {
        store.upsert(descriptor);
        store.save().map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// 서드파티 앱 descriptor 삭제 — `library_remove`의 대응. 이 앱을 참조하는 레시피가
/// 있어도(참조 무결성 검사 없음) 그냥 지운다 — 그 레시피는 "알 수 없는 서드파티 앱"
/// 오류로 남을 뿐, 임의 코드 실행 등 안전 위험은 없다(모듈 설명 참고).
#[tauri::command]
pub async fn third_party_app_remove(id: String) -> Result<bool, String> {
    let mut store = load_third_party_app_store().await?;
    tauri::async_runtime::spawn_blocking(move || {
        let removed = store.remove(&id);
        store.save().map_err(|e| e.to_string())?;
        Ok(removed)
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

pub(super) fn find_third_party_descriptor(
    app_id: &str,
) -> Result<Option<pengport_shared::actions::ThirdPartyAppDescriptor>, String> {
    Ok(known_third_party_apps()?.into_iter().find(|d| d.id == app_id))
}

/// 사용자가 설정 화면에서 지정한 override 경로 — third-party app 전부가 공유하는
/// 제네릭 저장소(`third_party_app_overrides.json`, `app_id → 경로` 맵). descriptor 를
/// 데이터 파일로 뺀 뒤 다시 보니, "설정이 어디 저장돼 있는가"도 애초에 앱마다 다를
/// 이유가 없었다 — override 로 저장하는 값은 항상 `PathBuf` 하나뿐이라(`DataRootLookupContext.user_override_root`
/// 와 정확히 같은 모양) 앱별 커스텀 스키마가 필요한 지점이 아니었다. 그래서
/// `resolve_known_third_party_app`의 `match descriptor.id { ... }`도 함께 사라짐 — 새
/// third-party 앱은 이제 정말로 descriptor 데이터 1건 추가만으로 끝난다.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ThirdPartyAppOverrides {
    #[serde(default)]
    overrides: std::collections::HashMap<String, PathBuf>,
}

fn third_party_overrides_path() -> Option<PathBuf> {
    super::paths::app_data_root().map(|d| d.join("third_party_app_overrides.json"))
}

fn load_third_party_overrides() -> ThirdPartyAppOverrides {
    let Some(path) = third_party_overrides_path() else { return Default::default() };
    let Ok(text) = std::fs::read_to_string(path) else { return Default::default() };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_third_party_overrides(overrides: &ThirdPartyAppOverrides) -> Result<(), String> {
    let path = third_party_overrides_path().ok_or_else(|| "%APPDATA% 미정".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("디렉터리 생성 실패: {e}"))?;
    }
    let text = serde_json::to_string_pretty(overrides).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("파일 쓰기 실패: {e}"))
}

pub(super) fn third_party_app_override_root(app_id: &str) -> Option<PathBuf> {
    load_third_party_overrides().overrides.get(app_id).cloned()
}

/// 사용자가 override 경로를 지정/해제(`root: None`이면 해제) — 대상 앱의
/// `exe_filename`이 그 폴더에 실제로 있는지로 검증(잘못 지정한 폴더가 조용히 저장돼
/// "탐지 실패"로만 나타나는 걸 방지, 옛 `set_prism_override`의 검증 로직과 동일).
pub(super) fn set_third_party_app_override(app_id: &str, root: Option<PathBuf>) -> Result<(), String> {
    let mut overrides = load_third_party_overrides();
    match root {
        Some(path) => {
            let descriptor = find_third_party_descriptor(app_id)?
                .ok_or_else(|| format!("알 수 없는 third-party app: {app_id}"))?;
            if !path.join(&descriptor.exe_filename).is_file() {
                return Err(format!(
                    "선택한 폴더에 {} 가 없습니다: {}",
                    descriptor.exe_filename,
                    path.display()
                ));
            }
            overrides.overrides.insert(app_id.to_string(), path);
        }
        None => {
            overrides.overrides.remove(app_id);
        }
    }
    save_third_party_overrides(&overrides)
}

/// descriptor 로 실제 위치를 해석 — 경로 탐색 알고리즘(`resolve_third_party_app`)과
/// override 저장 위치 둘 다 완전히 범용이라, 이 함수는 이제 앱별 분기가 하나도 없다.
pub(super) fn resolve_known_third_party_app(
    descriptor: &pengport_shared::actions::ThirdPartyAppDescriptor,
) -> Result<pengport_shared::actions::ResolvedThirdPartyApp, String> {
    let ctx = pengport_shared::actions::DataRootLookupContext {
        user_override_root: third_party_app_override_root(&descriptor.id),
        bundled_root: super::paths::bundled_third_party_root(&descriptor.id),
    };
    pengport_shared::actions::resolve_third_party_app(descriptor, &ctx)
        .ok_or_else(|| format!("{} 를 찾을 수 없습니다.", descriptor.id))
}

// ---------------------------------------------------------------------------
// 설정 화면(`ThirdPartyApps.tsx`)용 범용 커맨드 — 탐지/override 지정/bundled 삭제
// 셋 다 app_id 하나로 등록된 모든 third-party app 을 다룬다. 예전엔 이 셋이
// `detect_prism`/`set_prism_override`/`remove_bundled_prism`으로 Prism 이름이 박혀
// 있었지만, 실제로 하는 일이 처음부터 `resolve_known_third_party_app` 하나(완전
// 범용)를 얇게 감싸는 것뿐이었다 — 이름만 Prism 전용이고 알고리즘은 이미 범용이었던
// 마지막 지점. **다운로드(OOBE 자동 설치)는 예외** — 이건 앱마다 실제로 구현이
// 다르다(Prism 은 GitHub latest-release API, 다른 앱은 다른 방식일 수 있음)는 게
// `THIRD_PARTY_PLATFORM_MODEL.md`가 이미 확정한 남은 한계라 `download_prism`은
// 그대로 Prism 전용 커맨드로 남는다.
// ---------------------------------------------------------------------------

/// 등록된 third-party app 하나의 현재 위치 탐지(못 찾으면 `None`) — 설정 화면
/// 카드가 마운트/새로고침 시 호출.
#[tauri::command]
pub fn detect_third_party_app(
    app_id: String,
) -> Result<Option<pengport_shared::actions::ResolvedThirdPartyApp>, String> {
    let Some(descriptor) = find_third_party_descriptor(&app_id)? else {
        return Ok(None);
    };
    Ok(resolve_known_third_party_app(&descriptor).ok())
}

/// 사용자가 설정 화면에서 override 경로 지정/해제(빈 문자열이면 해제) — 검증(대상
/// exe 존재 확인, `set_third_party_app_override`)까지 마친 뒤 갱신된 위치를 재탐지해
/// 돌려준다.
#[tauri::command]
pub fn configure_third_party_app_override(
    app_id: String,
    root: String,
) -> Result<Option<pengport_shared::actions::ResolvedThirdPartyApp>, String> {
    let trimmed = root.trim();
    let value = if trimmed.is_empty() { None } else { Some(PathBuf::from(trimmed)) };
    set_third_party_app_override(&app_id, value)?;
    let descriptor = find_third_party_descriptor(&app_id)?
        .ok_or_else(|| format!("알 수 없는 third-party app: {app_id}"))?;
    Ok(resolve_known_third_party_app(&descriptor).ok())
}

/// PengPort 가 자동 다운로드해둔 전용 사본(Bundled)을 삭제 — 사용자가 다른 사본으로
/// 갈아탈 때. 삭제 후 재탐지 결과 반환.
#[tauri::command]
pub fn remove_bundled_third_party_app(
    app_id: String,
) -> Result<Option<pengport_shared::actions::ResolvedThirdPartyApp>, String> {
    if let Some(root) = super::paths::bundled_third_party_root(&app_id) {
        if root.exists() {
            std::fs::remove_dir_all(&root)
                .map_err(|e| format!("{app_id} 전용 사본 삭제 실패: {e}"))?;
        }
    }
    let descriptor = find_third_party_descriptor(&app_id)?
        .ok_or_else(|| format!("알 수 없는 third-party app: {app_id}"))?;
    Ok(resolve_known_third_party_app(&descriptor).ok())
}

// ---------------------------------------------------------------------------
// third-party app 실행 "준비 완료" 감시 — descriptor 의 `readiness_signal`(범용
// `pengport_shared::actions::ReadinessSignal`)을 해석해서 실제로 관찰하는 부분.
// Prism 이 이 신호가 필요한 이유는 "우연히 이렇게 동작해서"가 아니라 Prism 자체가
// 이 신호를 공식적으로 안 주기 때문(Prism 공식 문서 확인됨 — pre-launch/post-exit
// 훅만 있고 "게임이 지금 떴다" 훅은 없음) — 그래서 같은 상황(런처가 페이로드를 별도
// 자식 프로세스로 띄우고 그 사실을 알려주지 않음)에 놓인 다음 앱에도 재사용될 수
// 있는 범용 감시기로 분리했다. descriptor 가 `readiness_signal` 을 선언 안 하면 이
// 감시 자체를 안 하고 spawn 즉시 실행 중으로 취급(호출자 책임).
// ---------------------------------------------------------------------------

/// [`ReadinessSignal`]을 해석해서 감시 task 를 백그라운드로 띄운다 — 판별되면
/// `third_party_app:child_ready`(`{ recipeId }`) 이벤트를 emit. Prism 전용이던
/// `spawn_prism_instance`의 자식 프로세스 감시 로직을 여기로 옮기고 문자열(cmdline
/// 패턴)만 descriptor 데이터로 바꿨다 — 로직 자체는 안 바뀜.
///
/// `still_running`: 호출자가 자기 방식대로 "이 recipe_id 가 지금도 실행 중으로
/// 추적되고 있나"를 답해주는 콜백(예: `commands::prism`의 PID 추적 맵 조회) — 사용자가
/// 도중에 종료시키면 여기서 `false`가 나와 polling 을 조기 중단한다. 이 함수는
/// 추적을 어떻게/어디서 하는지 몰라도 되게(의존성 주입) 남겨둔다 — 그래야 정말로
/// Prism 을 몰라도 되는 범용 유틸리티로 남는다.
pub(super) fn watch_third_party_app_readiness(
    parent_pid: u32,
    recipe_id: String,
    signal: pengport_shared::actions::ReadinessSignal,
    still_running: impl Fn() -> bool + Send + Sync + 'static,
    app: tauri::AppHandle,
) {
    use pengport_shared::actions::ReadinessSignal;
    tauri::async_runtime::spawn(async move {
        match signal {
            ReadinessSignal::ChildProcessWindow { cmdline_contains } => {
                watch_child_process_window(parent_pid, &recipe_id, &cmdline_contains, &still_running, &app)
                    .await;
            }
        }
    });
}

/// `parent_pid`의 자식 중 cmdline 에 `cmdline_contains`가 포함된 프로세스가 나타나고,
/// 그 프로세스가 visible top-level window 를 가질 때까지 polling(1초 간격, 최대
/// 10분) — 옛 `commands::prism::watch_for_minecraft_child`와 동일 로직, 대상 문자열만
/// 파라미터화.
async fn watch_child_process_window(
    parent_pid: u32,
    recipe_id: &str,
    cmdline_contains: &str,
    still_running: &(impl Fn() -> bool + Send + Sync + 'static),
    app: &tauri::AppHandle,
) {
    use std::time::Duration;
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let mut sys = System::new();
    let refresh_kind = ProcessRefreshKind::new().with_cmd(UpdateKind::Always);

    let max_iterations = 600;
    for _ in 0..max_iterations {
        tokio::time::sleep(Duration::from_secs(1)).await;
        sys.refresh_processes_specifics(ProcessesToUpdate::All, refresh_kind);

        if !still_running() {
            return;
        }

        let parent = sysinfo::Pid::from_u32(parent_pid);
        let child_pid: Option<u32> = sys.processes().iter().find_map(|(pid, p)| {
            if p.parent() != Some(parent) {
                return None;
            }
            let matches = p
                .cmd()
                .iter()
                .any(|arg| arg.to_string_lossy().contains(cmdline_contains));
            matches.then(|| pid.as_u32())
        });

        if let Some(child_pid) = child_pid {
            #[cfg(windows)]
            let ready = has_visible_window_for_pid(child_pid);
            #[cfg(not(windows))]
            let ready = {
                let _ = child_pid;
                true
            };
            if ready {
                let _ = app.emit(
                    "third_party_app:child_ready",
                    serde_json::json!({ "recipeId": recipe_id }),
                );
                return;
            }
        }
    }
}

/// 특정 PID 가 visible top-level window 를 가지고 있는지. `EnumWindows`로 모든
/// top-level window 를 순회 + `GetWindowThreadProcessId`로 PID 매칭 — Prism 전용
/// 코드였으나 Prism/Minecraft 를 전혀 몰라서 범용 유틸리티로 여기로 옮김.
#[cfg(windows)]
fn has_visible_window_for_pid(target_pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
    };

    struct State {
        target_pid: u32,
        found: bool,
    }

    unsafe extern "system" fn cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = unsafe { &mut *(lparam as *mut State) };
        if unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == state.target_pid {
            state.found = true;
            return 0;
        }
        1
    }

    let mut state = State {
        target_pid,
        found: false,
    };
    unsafe {
        EnumWindows(Some(cb), &mut state as *mut _ as LPARAM);
    }
    state.found
}

/// third-party app 의 인스턴스(레시피 1개에 대응) 데이터 폴더 위치. 데이터 루트 해석
/// 규칙(`resolve_third_party_app`, 범용)만으로 완전히 결정된다 — 이 함수는 app_id 로
/// descriptor 를 찾아 넘기기만 할 뿐, 경로 계산 자체엔 앱별 분기가 없다.
pub(super) fn third_party_app_instance_dir(app_id: &str, instance_id: &str) -> Result<PathBuf, String> {
    let descriptor = find_third_party_descriptor(app_id)?
        .ok_or_else(|| format!("알 수 없는 third-party app: {app_id} (install steps 에서 데이터를 쓸 수 있는 대상이 아님)"))?;
    let resolved = resolve_known_third_party_app(&descriptor)?;
    let instances_subfolder = descriptor
        .instances_subfolder
        .as_deref()
        .ok_or_else(|| format!("{app_id}: instances_subfolder 미설정"))?;
    Ok(pengport_shared::actions::third_party_instance_dir(
        &resolved.data_root,
        instances_subfolder,
        instance_id,
    ))
}

/// 레시피의 `archives`/`files` 가 공유하는 유일한 대상 루트 — `launch` 하나가 결정한다
/// (`recipe.rs` 모듈 설명 참고). `SpawnProcess`면 앱 전용 폴더, `ThirdPartyAppLaunch`면
/// 그 앱의 인스턴스 데이터 영역.
fn resolve_target_root(recipe_id: &str, launch: &LaunchAction) -> Result<PathBuf, String> {
    match launch {
        LaunchAction::SpawnProcess { .. } => super::paths::app_root(recipe_id)
            .ok_or_else(|| "app_root 미정 (%LOCALAPPDATA% 환경변수 없음)".to_string()),
        LaunchAction::ThirdPartyAppLaunch { app_id } => third_party_app_instance_dir(app_id, recipe_id),
    }
}

fn spawn_local_process(root_dir: &Path, entry_point: &str, entry_args: &[String]) -> Result<(), String> {
    let entry_path = root_dir.join(entry_point);
    if !entry_path.is_file() {
        return Err(format!(
            "설치 후에도 실행 파일을 찾을 수 없음: {} (레시피의 entry_point 확인 필요)",
            entry_path.display()
        ));
    }
    let working_dir = entry_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root_dir.to_path_buf());

    std::process::Command::new(&entry_path)
        .args(entry_args)
        .current_dir(&working_dir)
        .spawn()
        .map_err(|e| format!("{} 실행 실패: {e}", entry_path.display()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// reconcile_install — 압축 전부(다운로드+검증+해제+화이트리스트 정리) → 오버라이드
// 전부, 순서대로 **원장에 없는 항목만** 적용한다. "설치"(전부 마커 없음)와
// "업데이트"(바뀐 항목만 마커 없음)가 같은 함수인 이유.
//
// 마커는 항목 내용(전체 구조 JSON)의 SHA256 해시를 파일명으로 써서 "이 정확한
// 항목을 마지막으로 성공적으로 적용했는지"만 추적한다 — 지금 실제 파일 내용을
// 다시 읽어 비교하지 않는다. 설치 이후 앱을 사용하면서 생기는 정상적인 변화(캐시,
// 사용자가 앱 안에서 바꾼 설정 등)를 "업데이트 필요"로 오판하지 않기 위한 의도적
// 선택(모듈 설명 참고) — 압축은 원래도 이 방식이었고, 오버라이드도 한 세션 안에서
// "실제 파일 재비교"로 바꿨다가 다시 이 방식으로 되돌렸다.
// 마커는 항상 레시피 전용 `apps/<id>/` 아래 둔다.
// ---------------------------------------------------------------------------

async fn reconcile_install(
    recipe: &Recipe,
    selected: &HashSet<String>,
    app: &tauri::AppHandle,
) -> Result<InstallOutcome, String> {
    let cancel_flag = register_install_cancel_flag(&recipe.id);
    let _cancel_guard = InstallCancelGuard(recipe.id.clone());

    let markers_dir = super::paths::app_root(&recipe.id)
        .ok_or_else(|| "app_root 미정 (%LOCALAPPDATA% 환경변수 없음)".to_string())?
        .join(".pengport-markers");
    let root = resolve_target_root(&recipe.id, &recipe.launch)?;

    // 이전 설치 시도가 중간에 죽었을 때 남을 수 있는 임시 다운로드 파일 정리 — 이
    // 폴더는 PengPort 가 이 레시피 전용으로만 쓰는 스크래치 공간이라 통째로 지워도 안전.
    // `.pengport-tmp-pending`(압축 해제 충돌 확인 대기 중인 보존 다운로드)은 여기서
    // 안 건드림 — 사용자가 아직 답 안 한 충돌이 있으면 재다운로드 없이 이어가야 하므로.
    if let Ok(tmp_dir) = archive_tmp_dir(&recipe.id) {
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
    if let Ok(pending_dir) = archive_pending_conflict_dir(&recipe.id) {
        prune_orphaned_pending_conflicts(recipe, &pending_dir);
    }

    let files_with_override: Vec<&RecipeFile> = effective_files(recipe, selected)
        .into_iter()
        .filter(|f| f.override_content.is_some())
        .collect();
    let archives_in_scope: Vec<&ArchiveExtraction> = recipe
        .archives
        .iter()
        .filter(|a| match &a.optional_group {
            None => true,
            Some(group) => selected.contains(group),
        })
        .collect();
    let total = files_with_override.len() + archives_in_scope.len();
    let mut updated = 0usize;
    let mut item_index = 0usize;

    // order 기준 정렬 — 배열에 적힌 순서가 아니라 이 값이 실제 실행 순서의 유일한
    // 근거(`validate_recipe`가 이미 값 중복 없음을 보장).
    let mut ordered_archives: Vec<&ArchiveExtraction> = recipe.archives.iter().collect();
    ordered_archives.sort_by_key(|a| a.order);

    // 같은 목적지를 공유하는 base 압축(optional_group/raw_filename 둘 다 없는 것 —
    // 이 둘은 이미 각자 다른 방식으로 dest_dir 를 통째로 신뢰하므로 이 그룹화 대상이
    // 아님)을 미리 그룹화 — 그중 하나라도 마커가 안 맞으면 그룹 전체를 order 순서대로
    // 재적용한다(방향성은 `archive_must_run` 참고).
    let mut archives_with_dirty: Vec<(&ArchiveExtraction, bool)> = Vec::with_capacity(ordered_archives.len());
    for archive in &ordered_archives {
        let hash = archive_content_hash(archive)?;
        archives_with_dirty.push((archive, !marker_exists(&markers_dir, &hash)));
    }
    let dirty_dest_dirs = dirty_shared_dest_dirs(&root, &archives_with_dirty);

    for archive in ordered_archives {
        let in_scope = match &archive.optional_group {
            None => true,
            Some(group) => selected.contains(group),
        };
        let hash = archive_content_hash(archive)?;

        if !in_scope {
            // 그룹 선택 해제됨 — 이전에 설치된 적 있으면(마커 존재) 콘텐츠+마커 정리.
            // 애초에 설치된 적 없으면 아무 것도 안 함(불필요한 디스크 접근 방지).
            if marker_exists(&markers_dir, &hash) {
                remove_grouped_archive_content(recipe, &root, &markers_dir, archive)?;
                updated += 1;
            }
            continue;
        }

        let dest_dir = merge_dest(&root, &archive.extract_to);
        let must_run = archive_must_run(archive, &dest_dir, &dirty_dest_dirs, !marker_exists(&markers_dir, &hash));

        item_index += 1;
        if !must_run {
            continue;
        }
        check_cancelled(&cancel_flag)?;
        emit_step_started(app, &recipe.id, item_index, total, "archive", &archive.url);
        let recipe_clone = recipe.clone();
        let archive_clone = archive.clone();
        let selected_clone = selected.clone();
        let app_clone = app.clone();
        let cancel_flag_clone = cancel_flag.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            execute_archive(&recipe_clone, &archive_clone, &selected_clone, &app_clone, &cancel_flag_clone)
        })
        .await
        .map_err(|e| format!("blocking task panic: {e}"))??;
        let written = match result {
            ArchiveExecutionResult::Written(written) => written,
            ArchiveExecutionResult::ConflictsPending(group) => {
                return Ok(InstallOutcome::HasArchiveConflicts { archives: vec![group] });
            }
        };
        write_marker(&markers_dir, &hash)?;
        if archive.optional_group.is_some() {
            write_manifest(&markers_dir, &hash, &written)?;
        }
        updated += 1;
        emit_step_completed(app, &recipe.id, item_index, total);
    }

    // 압축이 전부 끝난 뒤 폴더 옵션(`recipe.folder_rules`)을 독립적으로 적용 — 어떤
    // 압축이 그 폴더를 채웠는지(혹은 압축이 아예 없는지)와 무관하게 매 재설치마다
    // 평가되므로, 레시피 편집만으로(압축이 dirty 하지 않아도) 필터가 바로 반영된다.
    check_cancelled(&cancel_flag)?;
    let recipe_clone = recipe.clone();
    let selected_clone = selected.clone();
    let root_clone = root.clone();
    let markers_dir_clone = markers_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        apply_folder_rules(&recipe_clone, &selected_clone, &root_clone, &markers_dir_clone)
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))??;

    for file in files_with_override {
        let hash = file_override_hash(file)?;
        item_index += 1;
        if is_resolved(&markers_dir, &hash) {
            continue;
        }
        check_cancelled(&cancel_flag)?;
        emit_step_started(app, &recipe.id, item_index, total, "file", &file.path);
        let recipe_id = recipe.id.clone();
        let launch = recipe.launch.clone();
        let file_clone = file.clone();
        tauri::async_runtime::spawn_blocking(move || execute_override(&recipe_id, &launch, &file_clone))
            .await
            .map_err(|e| format!("blocking task panic: {e}"))??;
        write_marker(&markers_dir, &hash)?;
        updated += 1;
        emit_step_completed(app, &recipe.id, item_index, total);
    }

    Ok(InstallOutcome::Completed { updated, total })
}

/// 설치 진행률 이벤트 — 프론트가 `install:*` 을 `recipeId` 로 필터링해서 카드별
/// 진행 상태를 그린다. 기존 `server:started`(prism.rs) 등과 같은 컨벤션(camelCase
/// JSON, 타입 없이 `serde_json::json!`)을 그대로 따른다.
fn emit_step_started(app: &tauri::AppHandle, recipe_id: &str, index: usize, total: usize, kind: &str, label: &str) {
    let _ = app.emit(
        "install:step-started",
        serde_json::json!({ "recipeId": recipe_id, "index": index, "total": total, "kind": kind, "label": label }),
    );
}

fn emit_step_completed(app: &tauri::AppHandle, recipe_id: &str, index: usize, total: usize) {
    let _ = app.emit(
        "install:step-completed",
        serde_json::json!({ "recipeId": recipe_id, "index": index, "total": total }),
    );
}

fn marker_exists(markers_dir: &Path, hash: &str) -> bool {
    markers_dir.join(format!("{hash}.done")).exists()
}

fn write_marker(markers_dir: &Path, hash: &str) -> Result<(), String> {
    std::fs::create_dir_all(markers_dir).map_err(|e| format!("설치 마커 폴더 생성 실패: {e}"))?;
    std::fs::write(markers_dir.join(format!("{hash}.done")), "")
        .map_err(|e| format!("설치 마커 기록 실패: {e}"))
}

fn has_any_marker(markers_dir: &Path) -> bool {
    std::fs::read_dir(markers_dir)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

fn remove_marker(markers_dir: &Path, hash: &str) -> Result<(), String> {
    let path = markers_dir.join(format!("{hash}.done"));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("설치 마커 삭제 실패: {e}"))?;
    }
    Ok(())
}

fn declined_marker_path(markers_dir: &Path, hash: &str) -> PathBuf {
    markers_dir.join(format!("{hash}.declined"))
}

/// [`library_resolve_override_conflicts`]에서 사용자가 "업데이트하지 않기"를 고른
/// 파일에 기록 — 이 정확한 선언값(해시)에 대해선 다음 설치/업데이트에서 다시
/// 안 물어보고 조용히 건너뛴다. 레시피가 또 바뀌어 해시가 달라지면 새 해시는
/// declined 마커가 없으니 자연히 다시 판정 대상이 된다.
fn write_declined_marker(markers_dir: &Path, hash: &str) -> Result<(), String> {
    std::fs::create_dir_all(markers_dir).map_err(|e| format!("건너뜀 마커 폴더 생성 실패: {e}"))?;
    std::fs::write(declined_marker_path(markers_dir, hash), "")
        .map_err(|e| format!("건너뜀 마커 기록 실패: {e}"))
}

/// "이 해시는 더 이상 안 물어봐도/재적용 안 해도 됨" — 실제로 적용됐거나(`.done`),
/// 사용자가 명시적으로 건너뛰기를 골랐거나(`.declined`) 둘 중 하나. pending 판정과
/// [`detect_override_conflicts`]가 공유하는 단일 판정점.
fn is_resolved(markers_dir: &Path, hash: &str) -> bool {
    marker_exists(markers_dir, hash) || declined_marker_path(markers_dir, hash).exists()
}

fn path_fingerprint_dir(markers_dir: &Path) -> PathBuf {
    markers_dir.join("paths")
}

fn path_fingerprint_marker(markers_dir: &Path, file_path: &str) -> PathBuf {
    path_fingerprint_dir(markers_dir).join(format!("{}.content-sha256", sha256_hex(file_path.as_bytes())))
}

/// [`execute_override`]가 파일을 실제로 쓸 때마다 그 원본 바이트의 SHA256을
/// 기록해두는 "적용 지문" — "PengPort가 마지막으로 이 경로에 실제로 쓴 내용".
/// `file_override_hash`(선언값 자체의 해시, 빈 `.done` 마커)와는 별개다 — 그건
/// "이 선언값을 적용한 적 있는가"만 알고 디스크의 지금 상태는 전혀 모른다. 드리프트
/// 판정([`detect_override_conflicts`])은 이 지문과 지금 디스크 파일의 해시를
/// 비교해서 한다.
fn write_path_fingerprint(markers_dir: &Path, file_path: &str, content_bytes: &[u8]) -> Result<(), String> {
    let dir = path_fingerprint_dir(markers_dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("적용 지문 폴더 생성 실패: {e}"))?;
    std::fs::write(path_fingerprint_marker(markers_dir, file_path), sha256_hex(content_bytes))
        .map_err(|e| format!("적용 지문 기록 실패: {e}"))
}

/// 지문이 없으면(이 path를 이 메커니즘으로 관리한 적 없음 — 첫 설치 포함) `None`.
fn read_path_fingerprint(markers_dir: &Path, file_path: &str) -> Option<String> {
    std::fs::read_to_string(path_fingerprint_marker(markers_dir, file_path)).ok()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

// ---------------------------------------------------------------------------
// 그룹 압축 매니페스트 — [`execute_archive`]가 그룹 압축(`optional_group` 있음)을 풀
// 때 실제로 쓴 상대경로 목록을 기록해둔다. 그룹을 지울 때 `dest_dir`를 통째로
// 밀어버리는 대신 이 목록에 있는 파일만 정밀 삭제해서, 같은 폴더에 다른 압축(예:
// base 압축)이 넣어둔 콘텐츠를 안 건드리게 한다. 매니페스트가 없으면(레거시 설치,
// 또는 유실) [`remove_grouped_archive_content`]가 기존 방식(통째로 삭제 +
// [`invalidate_ancestor_markers`])으로 폴백한다 — "왜 없는지"는 구분하지 않는다.
// ---------------------------------------------------------------------------

fn manifest_path(markers_dir: &Path, hash: &str) -> PathBuf {
    markers_dir.join(format!("{hash}.manifest"))
}

fn write_manifest(markers_dir: &Path, hash: &str, paths: &[String]) -> Result<(), String> {
    std::fs::create_dir_all(markers_dir).map_err(|e| format!("매니페스트 폴더 생성 실패: {e}"))?;
    let json = serde_json::to_vec(paths).map_err(|e| format!("매니페스트 직렬화 실패: {e}"))?;
    std::fs::write(manifest_path(markers_dir, hash), json)
        .map_err(|e| format!("매니페스트 쓰기 실패: {e}"))
}

/// 매니페스트가 없거나 손상됐으면 `None` — 호출자가 "매니페스트 없음"과 동일하게
/// 취급해 통째 삭제 폴백으로 넘어간다(파싱 실패로 에러를 내는 대신).
fn read_manifest(markers_dir: &Path, hash: &str) -> Option<Vec<String>> {
    let bytes = std::fs::read(manifest_path(markers_dir, hash)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn remove_manifest_file(markers_dir: &Path, hash: &str) {
    let _ = std::fs::remove_file(manifest_path(markers_dir, hash));
}

/// 그룹 압축 하나의 콘텐츠+마커+매니페스트를 지운다. 매니페스트가 있으면 그 안에
/// 기록된 파일들만 정밀 삭제(다른 압축이 같은 폴더에 넣어둔 콘텐츠는 보존)하고 빈
/// 하위 폴더만 정리, 없으면(레거시 또는 유실) `dest_dir` 통째로 삭제 + 조상 압축
/// 마커 무효화로 폴백한다. [`reconcile_install`]의 그룹 선택 해제 정리와
/// `library_delete_installed_data`의 부분 삭제 둘 다 이 함수 하나를 공유한다.
fn remove_grouped_archive_content(
    recipe: &Recipe,
    root: &Path,
    markers_dir: &Path,
    archive: &ArchiveExtraction,
) -> Result<(), String> {
    let dest_dir = merge_dest(root, &archive.extract_to);
    let hash = archive_content_hash(archive)?;

    if let Some(paths) = read_manifest(markers_dir, &hash) {
        for rel in &paths {
            let full = dest_dir.join(rel);
            if full.exists() {
                std::fs::remove_file(&full)
                    .map_err(|e| format!("그룹 파일 삭제 실패 ({}): {e}", full.display()))?;
            }
        }
        if dest_dir.exists() {
            prune_empty_subdirs(&dest_dir, &[])?;
        }
        remove_manifest_file(markers_dir, &hash);
    } else if dest_dir.exists() {
        std::fs::remove_dir_all(&dest_dir)
            .map_err(|e| format!("그룹 콘텐츠 삭제 실패 ({}): {e}", dest_dir.display()))?;
        invalidate_ancestor_markers(recipe, root, markers_dir, &dest_dir)?;
    }

    remove_marker(markers_dir, &hash)
}

/// `wiped_dest_dir`를 통째로 지운 직후(그룹 선택 해제 또는 그룹별 삭제) 호출 — 그
/// 폴더 자체를 소유하지 않으면서도 그 안에 자기 콘텐츠를 두는 **조상** 압축들의
/// 마커를 무효화한다. base 압축(`extract_to`가 그룹 압축 목적지의 상위 경로)의 내부
/// 압축 구조가 그룹 압축과 같은 폴더를 공유할 수 있다 — 그룹 폴더를 통째로 지우면
/// base 가 넣어둔 몫도 같이 사라지는데, base 자신의 마커는 안 바뀌었으니 다음
/// 재설치 때도 자동 복원이 안 된다. 마커를 지워두면 다음 설치/업데이트 때 그 조상
/// 압축이 다시 실행돼 자기 몫을 복원한다.
///
/// **형제 그룹(GroupB 등)까지 번지지 않는다** — 정확히 "조상"(같거나 상위 폴더) 관계만
/// 보고, 다른 그룹의 dest_dir는 서로 조상 관계가 아니므로 영향받지 않는다. `raw_filename`
/// 압축도 대상에 포함 — 단일 파일이 지워진 폴더 안에 있었을 수 있어 같은 논리로 복원
/// 대상이다.
fn invalidate_ancestor_markers(
    recipe: &Recipe,
    root: &Path,
    markers_dir: &Path,
    wiped_dest_dir: &Path,
) -> Result<(), String> {
    for archive in ancestor_archives(recipe, root, wiped_dest_dir) {
        remove_marker(markers_dir, &archive_content_hash(archive)?)?;
    }
    Ok(())
}

/// [`invalidate_ancestor_markers`]의 판단 로직만 분리한 순수 함수 — `wiped_dest_dir`를
/// 소유하지 않으면서도(경로가 다름) 그 상위 폴더를 목적지로 삼는 압축들을 고른다.
/// I/O(마커 파일 존재/삭제)가 없어 결정론적으로 테스트 가능.
fn ancestor_archives<'a>(
    recipe: &'a Recipe,
    root: &Path,
    wiped_dest_dir: &Path,
) -> Vec<&'a ArchiveExtraction> {
    recipe
        .archives
        .iter()
        .filter(|archive| {
            let dest_dir = merge_dest(root, &archive.extract_to);
            dest_dir != wiped_dest_dir && wiped_dest_dir.starts_with(&dest_dir)
        })
        .collect()
}

/// 압축 내용물의 정체성만 해시(URL/검증값/해제 위치) — **선택 그룹 상태는 포함 안
/// 함**. "지금 이 압축이 유효 범위인가"(그룹 선택 여부)는 마커 정체성과 무관한 별도
/// 축이라 [`reconcile_install`]이 직접 판단한다(포함시키면 무관한 다른 그룹을 토글할
/// 때마다 이 압축까지 "바뀐 것"으로 오판해 불필요한 재다운로드가 발생함).
///
/// **`ArchiveExtraction` 구조체를 통째로 직렬화하지 않고, 의미있는 필드만 명시적으로
/// 골라서 해시한다** — 구조체 전체를 해시하면 Rust 쪽에 (마커 판정과 무관한) 필드가
/// 하나 추가되기만 해도 모든 기존 마커가 깨져서 전체 재다운로드가 강제된다(실제로
/// 이번 세션에 선택 그룹 기능을 추가하며 한 번 겪은 버그). 마커 해시는 "레시피 작성자가
/// 실제로 바꿀 수 있는 값"에만 반응해야 한다.
fn archive_content_hash(archive: &ArchiveExtraction) -> Result<String, String> {
    // `path_overrides`도 포함 — `extract_to`와 같은 이유(실제 파일이 어디에 놓이는지를
    // 바꾸는 값이라 바뀌면 재적용돼야 함).
    hash_json(&(
        &archive.url,
        &archive.verification,
        &archive.extract_to,
        &archive.path_overrides.iter().map(|o| (&o.from, &o.to)).collect::<Vec<_>>(),
    ))
}

/// `RecipeFile` 통째로가 아니라 `path`+`override_content`만 해시 — 위 아카이브 해시와
/// 같은 이유. `optional_group`은 "이 파일이 지금 유효한가"(포함 여부, effective_files가
/// 이미 처리)를 결정할 뿐 오버라이드 내용 자체와 무관하므로 마커 정체성에 안 넣는다.
fn file_override_hash(file: &RecipeFile) -> Result<String, String> {
    hash_json(&(&file.path, &file.override_content))
}

fn hash_json<T: serde::Serialize>(value: &T) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let json = serde_json::to_string(value).map_err(|e| format!("직렬화 실패: {e}"))?;
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

/// `recipe.files` 중 지금 유효한 것만 — `optional_group`이 없으면 항상 포함, 있으면
/// `selected`에 있을 때만. 화이트리스트/오버라이드 적용/상태 조회 전부 이 함수 하나를
/// 공유한다(레시피가 아는 전체 파일 목록과, "지금 이 사용자에게 유효한" 부분집합을
/// 뒤섞지 않기 위함).
fn effective_files<'a>(recipe: &'a Recipe, selected: &HashSet<String>) -> Vec<&'a RecipeFile> {
    recipe
        .files
        .iter()
        .filter(|f| match &f.optional_group {
            None => true,
            Some(group) => selected.contains(group),
        })
        .collect()
}

/// `optional_group`이 있는 압축들이 실제로 해제되는 위치. 이 경로들은 [`execute_archive`]가
/// 통째로 소유·관리하므로(개별 `RecipeFile` 화이트리스트 없이 전체 신뢰), 그룹 없는
/// (베이스) 압축의 화이트리스트 정리(`prune_disallowed_files`)가 여길 건드리면 안 된다 —
/// 그래서 그 정리 단계에 exclude 목록으로 넘긴다.
fn grouped_archive_dest_dirs(recipe: &Recipe, root: &Path) -> Vec<PathBuf> {
    recipe
        .archives
        .iter()
        .filter(|a| a.optional_group.is_some())
        .map(|a| merge_dest(root, &a.extract_to))
        .collect()
}

/// 레시피가 참조하는 third-party app id — `launch` 하나가 `archives`/`files`의 대상
/// 루트도 결정하므로, 이제 참조는 항상 최대 1개(`recipe.rs` 모듈 설명 참고).
fn referenced_third_party_app_ids(recipe: &Recipe) -> Vec<String> {
    match &recipe.launch {
        LaunchAction::ThirdPartyAppLaunch { app_id } => vec![app_id.clone()],
        LaunchAction::SpawnProcess { .. } => Vec::new(),
    }
}

/// third-party app 이 지금 시스템에서 탐지되는지 — `third_party_app_instance_dir`과
/// 같은 descriptor 조회 + 범용 해석을 재사용.
fn check_third_party_app_available(app_id: &str) -> Result<(), String> {
    let descriptor = find_third_party_descriptor(app_id)?
        .ok_or_else(|| format!("알 수 없는 third-party app: {app_id}"))?;
    resolve_known_third_party_app(&descriptor).map(|_| ())
}

/// 압축 다운로드+검증+해제.
///
/// `optional_group`이 없는(베이스) 압축은 기존 방식 그대로 — 해제 직후 `extract_to`
/// 범위 안에서 실제로 나온 모든 파일을 **지금 선택 상태 기준 유효한**
/// 화이트리스트([`effective_files`])와 대조해서, 선언 안 됐거나 선택 안 된 그룹의
/// 파일은 즉시 삭제한다(예외 없음, 단 [`grouped_archive_dest_dirs`]가 관리하는
/// 하위트리는 건드리지 않음).
///
/// `optional_group`이 있는(콘텐츠 팩) 압축은 **폴더 자체가 화이트리스트** — 개별
/// 파일을 `RecipeFile`로 일일이 선언하지 않는다(콘텐츠 팩 하나에 수천 개 파일이
/// 들어있어 파일 단위 화이트리스트가 실질적으로 불가능한 규모). `extract_to`는 이
/// 압축의 전용 리프 경로를 가리킨다(예: `SampleApp/Content`) — 압축 내부의 최상위
/// 폴더(보통 자기 이름과 같음, 예: `Content/file.bin`)는 [`extract_archive_file`]이
/// 벗겨내고 곧장 그 리프 밑에 푼다.
///
/// **덮어쓰기 병합이지 wipe-and-reinstall이 아니다** — 해제 전 기존 내용을 지우지
/// 않는다. `dest_dir`가 third-party app 등 다른 프로세스가 같이 관리하는 폴더일 수
/// 있어, "이 폴더 자체가 화이트리스트"라는 원칙상 폴더 안 기존 파일을 이 압축이
/// 함부로 지우면 안 된다. 그룹을 완전히 끌 때(사용자가 명시적으로 선택 해제)만
/// [`reconcile_install`]의 별도 경로가 `dest_dir`를 통째로 삭제한다 — 그건 "이 그룹
/// 자체를 원치 않는다"는 명시적 의사표시라 다른 성격의 삭제다. zip-slip 방어는
/// 화이트리스트가 아니라 [`safe_join_archive_entry`](zip/7z 공용)가 담당하므로
/// 화이트리스트를 생략해도 경로 탈출 위험은 없다.
///
/// 다운로드는 청크 단위로 임시 파일에 스트리밍한다(전체를 메모리에 담지 않음 —
/// 콘텐츠 팩처럼 개별 아카이브가 수 GB 인 실사용 콘텐츠가 있어 `Vec<u8>` 버퍼링은
/// 자원 고갈 위험이 있었다, `docs/design/INSTALL_PROGRESS.md` 참고). 압축 해제도 그
/// 임시 파일을 `Read + Seek` 로 그대로 열어 처리 — 압축 해제 라이브러리(zip/7z) 자체가
/// 메모리에 더 담을 수 있는지는 이 함수의 책임 범위 밖.
/// [`execute_archive`]의 결과 — 정상적으로 다 썼는지, 아니면 압축 해제 충돌이
/// 발견돼 사용자 확인이 필요해서 이번 시도를 멈췄는지.
enum ArchiveExecutionResult {
    Written(Vec<String>),
    ConflictsPending(ArchiveConflictGroup),
}

fn resolution_path(r: &ArchiveEntryResolution) -> &str {
    match r {
        ArchiveEntryResolution::Overwrite { path } => path,
        ArchiveEntryResolution::Skip { path } => path,
        ArchiveEntryResolution::Rename { path } => path,
    }
}

fn execute_archive(
    recipe: &Recipe,
    archive: &ArchiveExtraction,
    selected: &HashSet<String>,
    app: &tauri::AppHandle,
    cancel_flag: &AtomicBool,
) -> Result<ArchiveExecutionResult, String> {
    let hash = archive_content_hash(archive)?;
    let pending_dir = archive_pending_conflict_dir(&recipe.id)?;
    let pending_path = pending_download_path(&pending_dir, &hash);

    // 이전 시도가 압축 해제 충돌 확인 대기 중에 남겨둔 검증된 다운로드가 있으면
    // 재사용(재검증 후) — 대용량 압축을 다시 받지 않기 위함. 검증 실패(손상/유실)면
    // 지우고 평소대로 새로 받는다.
    let tmp_path: PathBuf =
        if pending_path.exists() && hash_file_and_verify(&pending_path, &archive.verification).is_ok() {
            pending_path.clone()
        } else {
            if pending_path.exists() {
                let _ = std::fs::remove_file(&pending_path);
            }
            let tmp_dir = archive_tmp_dir(&recipe.id)?;
            let download_throttle = Throttle::new(Duration::from_millis(150));
            let recipe_id = recipe.id.clone();
            let label = archive.url.clone();
            // 항상 먼저 직접 받아본다 — 실제로 압축/파일이 아니라 사람이 눌러야 하는
            // 페이지가 돌아온 경우에만(`DownloadOutcome::InteractivePage`) 브라우저로
            // 폴백. 레시피 작성자가 "이건 직링크가 아니다"를 미리 선언할 필요가 없다
            // (예전엔 `ArchiveExtraction.browser_assisted` 플래그로 선언해야 했으나,
            // 시도해보면 바로 알 수 있는 사실이라 폐기 — 자세한 경위는 shared 크레이트
            // 쪽 옛 필드 doc 이력 참고 대신 이 함수가 SSOT).
            match download_and_verify_to_file(
                &archive.url,
                &archive.verification,
                "설치 아티팩트",
                Duration::from_secs(1800),
                &tmp_dir,
                Some(cancel_flag),
                |downloaded, total_bytes| {
                    if download_throttle.allow() || total_bytes.is_some_and(|t| downloaded >= t) {
                        let _ = app.emit(
                            "install:download-progress",
                            serde_json::json!({
                                "recipeId": recipe_id, "label": label,
                                "downloadedBytes": downloaded, "totalBytes": total_bytes
                            }),
                        );
                    }
                },
            )? {
                DownloadOutcome::Downloaded(path) => path,
                DownloadOutcome::InteractivePage => {
                    obtain_via_browser_assisted_download(recipe, archive, app, cancel_flag, &tmp_dir)?
                }
            }
        };

    let root = resolve_target_root(&recipe.id, &recipe.launch)?;
    let dest_dir = merge_dest(&root, &archive.extract_to);
    let raw_filename = archive.raw_filename.as_deref();
    // strip_root(최상위 컴포넌트 하나 벗기기)는 그룹 전용 콘텐츠 압축에서만, "압축
    // 내부가 통째로 자기 소유 폴더 하나로 감싸져 있다"(예: GroupA.7z 안이 전부
    // `GroupA/...`)는 전제에서만 옳다 — `path_overrides`를 선언한 압축은 그 전제 자체가
    // 다르다(파일들이 압축 최상위에 개별로 있고, 각 파일을 어디로 보낼지 직접
    // 지정하는 방식이라 감싸는 폴더가 없음). 이 경우 strip_root=true 를 그대로 쓰면
    // 최상위 파일 엔트리(경로 부품이 하나뿐)가 "감싸는 폴더 이름 자체를 가리키는
    // 엔트리"로 오인돼 `safe_join_archive_entry`가 통째로 건너뛰어버린다 — 실제로 이
    // 버그로 SampleApp 레시피의 개별 파일 교체용 그룹 압축들이 압축을 열어도 아무 것도
    // 추출하지 못하고 있었다(2026-08 확인). `path_overrides`가 있으면 배치를 전적으로
    // 그게 담당하므로 strip_root 없이 원래 경로 그대로 추출한다.
    let strip_root = archive.optional_group.is_some() && archive.path_overrides.is_empty();

    // 이미 사용자가 해결한 충돌이 있으면(재시도) 그대로 적용 — 없으면 새로 스캔해서
    // 판정한다. 충돌이 있는데 아직 해결 안 됐으면 다운로드를 보존해두고 여기서 멈춘다.
    let resolutions: HashMap<String, ArchiveEntryResolution> = match read_pending_resolutions(&pending_dir, &hash) {
        Some(list) => list.into_iter().map(|r| (resolution_path(&r).to_string(), r)).collect(),
        None => {
            let scanned = scan_archive_entries(&tmp_path, &dest_dir, strip_root, raw_filename)?;
            let scanned = apply_path_overrides_to_scan(scanned, &root, &archive.path_overrides);
            let conflicts = detect_archive_conflicts(&scanned, recipe, &root);
            if !conflicts.is_empty() {
                if tmp_path != pending_path {
                    std::fs::create_dir_all(&pending_dir).map_err(|e| format!("대기 폴더 생성 실패: {e}"))?;
                    if std::fs::rename(&tmp_path, &pending_path).is_err() {
                        std::fs::copy(&tmp_path, &pending_path).map_err(|e| format!("보존 다운로드 복사 실패: {e}"))?;
                        let _ = std::fs::remove_file(&tmp_path);
                    }
                }
                return Ok(ArchiveExecutionResult::ConflictsPending(ArchiveConflictGroup {
                    archive_hash: hash,
                    url: archive.url.clone(),
                    conflicts,
                }));
            }
            HashMap::new()
        }
    };

    let _tmp_guard = TempFileGuard(tmp_path.clone());

    if archive.optional_group.is_some() {
        // 폴더 자체가 화이트리스트 — wipe 하지 않고 병합만. 그룹을 완전히 끄는 삭제는
        // `reconcile_install`의 별도(명시적) 경로가 담당(위 doc comment 참고). 반환하는
        // 상대경로 목록은 호출자가 매니페스트로 기록해서, 나중에 이 그룹만 정밀 삭제할
        // 때 다른 압축이 같은 폴더에 넣어둔 콘텐츠를 안 건드리게 한다.
        let ctx = ExtractProgressContext { app, recipe_id: &recipe.id, resolutions: &resolutions };
        let written = extract_archive_file(
            &tmp_path, &archive.url, &dest_dir, strip_root, raw_filename, ctx, Some(cancel_flag),
        )?;
        let result = apply_path_overrides(&written, &dest_dir, &root, &archive.path_overrides)?;
        remove_pending_conflict_files(&pending_dir, &hash);
        return Ok(ArchiveExecutionResult::Written(result));
    }

    let ctx = ExtractProgressContext { app, recipe_id: &recipe.id, resolutions: &resolutions };
    let written = extract_archive_file(
        &tmp_path, &archive.url, &dest_dir, false, raw_filename, ctx, Some(cancel_flag),
    )?;
    remove_pending_conflict_files(&pending_dir, &hash);

    if !archive_owns_dest_dir_whitelist(archive) {
        return Ok(ArchiveExecutionResult::Written(written));
    }

    let written = apply_path_overrides(&written, &dest_dir, &root, &archive.path_overrides)?;

    let allowed: HashSet<String> = effective_files(recipe, selected)
        .into_iter()
        .map(|f| f.path.clone())
        .collect();
    let mut exclude = grouped_archive_dest_dirs(recipe, &root);
    exclude.extend(pengport_internal_dirs(&recipe.id));
    exclude.extend(folder_rule_dest_dirs(recipe, &root));
    prune_disallowed_files(&dest_dir, &archive.extract_to, &allowed, &exclude)?;
    Ok(ArchiveExecutionResult::Written(written))
}

/// `recipe.folder_rules`에 선언된 모든 폴더의 절대 경로 — 모드나 재적용 시점과
/// 무관하게, 선언된 폴더는 전부 [`apply_folder_rules`]의 전속 관할이라 다른 어떤
/// 압축의 화이트리스트 정리(base 든 grouped 든)도 이 하위트리를 건드리면 안 된다.
/// 압축은 그 폴더를 누가 채웠는지와 무관하게 그냥 "여기에 풀기"만 할 뿐 — 그 안의
/// 내용을 실제로 어떻게 정리할지는 순전히 이 선언(레시피의 폴더 옵션)의 몫이다.
fn folder_rule_dest_dirs(recipe: &Recipe, root: &Path) -> Vec<PathBuf> {
    recipe.folder_rules.iter().map(|r| root.join(&r.path)).collect()
}

/// `recipe.folder_rules`를 독립적으로 적용 — 어떤 압축이 그 폴더를 채웠는지(혹은
/// 압축이 아예 없는지)와 무관하게, 선언된 폴더 경로의 **지금 실제 파일시스템
/// 내용**을 규칙대로 정리한다. [`reconcile_install`]에서 모든 압축이 끝난 뒤 한 번만
/// 호출된다.
///
/// **설치까지만 관여, 설치 후 앱 사용으로 생기는 변화는 건드리지 않는다**는
/// 원칙(`shared::library::recipe` 모듈 설명)에 따라 `Filtered`도 이 규칙(경로+패턴)
/// 자체가 바뀌지 않는 한 딱 1회만 정리한다 — 매 재설치마다 계속 강제하는 모드는
/// 없다(압축이 나중에 다시 받아져도 폴더 규칙 자체를 안 바꿨으면 재정리 안 함).
/// 마커는 다른 압축/오버라이드 마커와 같은 방식(규칙 내용 해시 — 규칙이 바뀌면
/// 해시도 바뀌어 다시 1회 적용됨).
///
/// `Passthrough`는 여기서도 할 일이 없다(다른 압축들의 pruning 이 이미
/// [`folder_rule_dest_dirs`]로 이 경로를 피해가므로 — 그게 전체 허용의 전부).
/// `Filtered`만 실제로 지운다: 선언된 `RecipeFile` 경로 ∪ (이 폴더 기준 상대 글롭
/// `patterns`에 걸리되 `disallow_patterns`에는 안 걸리는 파일) 만 남기고 나머지는 삭제.
/// `disallow_patterns`는 `declared`(명시적으로 선언된 파일)는 절대 못 지운다 — 폴더
/// 단위의 넓은 제외 패턴이 실수로 명시 등록 파일까지 삼키는 걸 막기 위한 설계
/// (`recipe.rs`의 `FolderRuleMode::Filtered` 문서 참고).
fn apply_folder_rules(recipe: &Recipe, selected: &HashSet<String>, root: &Path, markers_dir: &Path) -> Result<(), String> {
    let declared: HashSet<String> = effective_files(recipe, selected)
        .into_iter()
        .map(|f| f.path.clone())
        .collect();
    for rule in &recipe.folder_rules {
        let FolderRuleMode::Filtered { patterns, disallow_patterns } = &rule.mode else { continue };
        let hash = hash_json(rule)?;
        if marker_exists(markers_dir, &hash) {
            continue;
        }
        let folder_abs = root.join(&rule.path);
        if !folder_abs.is_dir() {
            continue; // 아직 아무도 안 채웠으면 정리할 것도 없음 — 다음 호출에서 재평가.
        }
        let compile = |pats: &BTreeSet<String>| -> Result<Vec<glob::Pattern>, String> {
            pats.iter()
                .map(|p| {
                    glob::Pattern::new(p)
                        .map_err(|e| format!("folder_rules: '{}' 의 패턴 '{p}' 파싱 실패: {e}", rule.path))
                })
                .collect()
        };
        let allow_compiled = compile(patterns)?;
        let disallow_compiled = compile(disallow_patterns)?;

        let mut leaves = Vec::new();
        collect_leaf_paths(&folder_abs, &[], &mut leaves)?;
        for file_path in leaves {
            let rel_from_folder = file_path
                .strip_prefix(&folder_abs)
                .map_err(|e| format!("경로 계산 실패 ({}): {e}", file_path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            let full_rel = format!("{}/{rel_from_folder}", rule.path);
            let allowed_by_pattern = allow_compiled.iter().any(|p| p.matches(&rel_from_folder))
                && !disallow_compiled.iter().any(|p| p.matches(&rel_from_folder));
            if declared.contains(&full_rel) || allowed_by_pattern {
                continue;
            }
            std::fs::remove_file(&file_path)
                .map_err(|e| format!("folder_rules 패턴에 안 맞는 파일 삭제 실패 ({}): {e}", file_path.display()))?;
        }
        prune_empty_subdirs(&folder_abs, &[])?;
        write_marker(markers_dir, &hash)?;
    }
    Ok(())
}

/// `archive.path_overrides`를 적용한다 — 압축 내부 구조가 평평해서(원래 서로 다른
/// 위치에 있던 파일들을 편의상 한 압축에 모아놓은 경우 등) `extract_to` 하나로는
/// 표현 못 하는 재배치를 "이 압축 안의 이 경로는 저 위치로 간다"처럼 명시적으로
/// 처리한다(사용자 요청 — 자동 추측이 아니라 항상 명시적 지정).
///
/// `from`은 두 가지로 매치된다(파일 하나 지정과 폴더 통째 지정을 같은 필드로 표현):
/// - **정확히 일치** — 파일 하나만 그 자리로 옮김(기존 동작 그대로).
/// - **`{from}/`로 시작(접두사)** — 그 폴더 밑 전부를, `from` 대신 `to`를 새 접두사로
///   붙여 그대로 옮김(예: `from: "A", to: ""` 면 `A/설치파일` → `설치파일`,
///   `A/sub/x.txt` → `sub/x.txt`). 압축 안 폴더 구조가 레시피가 원하는 구조와 아예
///   다를 때, 파일 수만큼 항목을 만들지 않아도 되게 하기 위함(2026-08, 사용자 확인).
///   정확히 일치가 항상 우선(더 구체적인 예외가 폴더 규칙보다 이긴다).
///
/// `written`(방금 추출된, `dest_dir` 기준 상대경로 목록) 중 매치 안 되는 항목은
/// 그대로 둔다. 파일을 옮기고 남은 빈 폴더(예: 폴더 통째 재배치 후의 원래 폴더) 자체는
/// 여기서 안 지운다 — 호출자(`execute_archive`)가 바로 다음에 돌리는
/// [`prune_disallowed_files`]가 끝에서 [`prune_empty_subdirs`]로 정리한다.
///
/// grouped(optional_group 있는) 압축에는 적용하지 않는다(호출자가 그 분기 이전에
/// early return) — 그쪽은 매니페스트가 `dest_dir` 기준 상대경로만 추적하는데,
/// override로 `dest_dir` 밖으로 옮겨진 파일은 그 모델을 벗어나 정밀 삭제 대상에서
/// 빠지게 된다. 지금 실제로 필요한 시나리오(base 압축)가 아니라 미리 다루지 않는다.
fn apply_path_overrides(
    written: &[String],
    dest_dir: &Path,
    root: &Path,
    overrides: &[PathOverride],
) -> Result<Vec<String>, String> {
    if overrides.is_empty() {
        return Ok(written.to_vec());
    }
    let mut result = Vec::with_capacity(written.len());
    for rel in written {
        let Some(new_to) = resolve_path_override_target(rel, overrides) else {
            result.push(rel.clone());
            continue;
        };
        let src = dest_dir.join(rel);
        let dest = root.join(&new_to);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("재배치 대상 폴더 생성 실패 ({}): {e}", parent.display()))?;
        }
        std::fs::rename(&src, &dest)
            .map_err(|e| format!("파일 재배치 실패 ({} → {}): {e}", src.display(), dest.display()))?;
        // 새 위치가 여전히 dest_dir 안이면(대부분의 경우 — extract_to가 짧은 접두어일
        // 뿐 목적지가 그 하위 폴더인 경우) 상대경로로 다시 표현해서 화이트리스트
        // 정리 대상에 정상적으로 포함시킨다. dest_dir 밖으로 완전히 나간 경우는 이
        // 압축의 pruning 대상이 아니게 되므로 목록에서 그냥 빠진다(문제 없음 — 그
        // 위치는 이 압축이 아니라 그 파일이 실제로 있는 폴더를 다루는 다른
        // 압축/규칙의 화이트리스트 대상일 뿐).
        if let Ok(new_rel) = dest.strip_prefix(dest_dir) {
            result.push(new_rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(result)
}

/// `rel`에 적용할 `path_overrides` 항목을 찾아 새 목적지 경로를 계산한다. 정확히
/// 일치가 항상 먼저(배열 순서와 무관) — 폴더 규칙 안의 파일 하나만 예외로 다른 곳에
/// 보내고 싶을 때, 그 예외가 폴더 규칙보다 뒤에 선언돼 있어도 이기게 하기 위함.
///
/// 폴더 단위 매칭(`from`이 파일 하나와 정확히 안 겹칠 때)은 rsync/cp의 트레일링
/// 슬래시 관례를 그대로 따른다(2026-08, 사용자 확인): `"GroupA/"`(슬래시 있음)는
/// 내용만 옮기고 `GroupA` 폴더 이름 자체는 사라짐(strip), `"GroupA"`(슬래시 없음)는
/// 폴더 이름을 유지한 채 `to` 밑에 통째로 얹는다(`to/GroupA/...`) — "폴더 이름을
/// 남길지"를 새 필드 없이 슬래시 유무 하나로 표현.
///
/// 어느 것에도 안 걸리면 `None`(호출자가 원래 경로 그대로 둠).
fn resolve_path_override_target(rel: &str, overrides: &[PathOverride]) -> Option<String> {
    if let Some(exact) = overrides.iter().find(|o| o.from == rel) {
        return Some(exact.to.clone());
    }
    overrides.iter().find_map(|o| {
        let strip = o.from.ends_with('/');
        let from = o.from.trim_end_matches('/');
        if from.is_empty() {
            return None; // 빈 from(전체 매치)은 지원 안 함 — extract_to 로 이미 표현 가능.
        }
        let suffix = rel.strip_prefix(from)?.strip_prefix('/')?;
        if strip {
            Some(if o.to.is_empty() {
                suffix.to_string()
            } else {
                format!("{}/{suffix}", o.to)
            })
        } else {
            // 폴더 이름(`from`) 유지 — `rel`이 이미 `{from}/{suffix}` 그대로라 통째로
            // `to` 밑에 다시 둔다.
            Some(if o.to.is_empty() { rel.to_string() } else { format!("{}/{rel}", o.to) })
        }
    })
}

/// PengPort 자신의 북키핑 폴더(`.pengport-markers`, `.pengport-tmp`) — 어떤 압축의
/// `dest_dir`에도 화이트리스트 정리 대상으로 포함되면 안 된다. `SpawnProcess` 레시피는
/// `target_root == app_root`라(모듈 상단 doc comment 참고) `extract_to`가 루트를
/// 가리키는 압축의 화이트리스트 정리 범위가 이 폴더까지 겹칠 수 있다 — `Recipe.files`엔
/// 마커 파일이 선언돼 있을 리 없어서 그대로 두면 "선언 안 된 파일"로 오인돼 삭제된다.
/// `ThirdPartyAppLaunch` 레시피는 `app_root`가 target_root와 별도 위치라 애초에 겹칠
/// 일이 없지만, 매번 계산해서 넘겨도 무해하다(`exclude`에 없는 경로는 그냥 무시됨).
fn pengport_internal_dirs(recipe_id: &str) -> Vec<PathBuf> {
    match super::paths::app_root(recipe_id) {
        Some(app_root) => vec![app_root.join(".pengport-markers"), app_root.join(".pengport-tmp")],
        None => Vec::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    Zip,
    SevenZ,
}

/// `extract_archive_file`/`scan_archive_entries`의 형식 판별 — 예전엔 URL 확장자(직접
/// 다운로드) 또는 브라우저로 받은 실제 파일명(확장자 보존)에 의존했는데, 압축 해제
/// 충돌로 다운로드를 보존했다가 나중에 재사용할 때는 파일명을 그대로 못 지킨다
/// (`{hash}.download`로 저장 — 원래 이름/URL과 무관). 이름 대신 파일 내용의 매직
/// 바이트로 직접 판별하면 이런 이름 유실과 무관하게 항상 정확하고, 애초에
/// "직접 다운로드 vs 브라우저 보조"라는 경로 분기 자체가 판별에 필요 없어진다.
fn sniff_archive_kind(path: &Path) -> Result<ArchiveKind, String> {
    use std::io::Read;
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("임시 파일 열기 실패 ({}): {e}", path.display()))?;
    let mut magic = [0u8; 6];
    let n = file
        .read(&mut magic)
        .map_err(|e| format!("임시 파일 읽기 실패 ({}): {e}", path.display()))?;
    if n >= 4 && magic[..4] == [0x50, 0x4B, 0x03, 0x04] {
        Ok(ArchiveKind::Zip)
    } else if n >= 6 && magic == [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C] {
        Ok(ArchiveKind::SevenZ)
    } else {
        Err("지원하지 않는 아카이브 형식(.zip/.7z 만 지원)".to_string())
    }
}

/// 사용자가 "직접 파일 선택"으로 지정한 파일을 옮겨두는 곳 — `execute_archive`가
/// 여는 감시 폴더 목록에도 이 폴더가 포함되므로([`obtain_via_browser_assisted_download`]),
/// [`library_stage_manual_archive_file`]가 여기에 파일을 갖다두기만 하면 이미 돌고
/// 있는 감시 루프가 다음 tick에 알아서 해시를 대조한다 — 별도의 "재시도" 신호가
/// 필요 없다.
fn manual_staging_dir(tmp_dir: &Path) -> PathBuf {
    tmp_dir.join("manual")
}

/// 직접 다운로드로 안 되는 것으로 판명된(`download_and_verify_to_file`이
/// `DownloadOutcome::InteractivePage`를 반환한) 압축의 폴백 — `archive.url`을
/// fetch하지 않고 기본 브라우저로 열어준 뒤, 다운로드 폴더를 감시해
/// `archive.verification` 해시와 일치하는 파일을 찾아 이 아카이브 전용 임시
/// 폴더로 **복사**해온다
/// (`browser_download::watch_for_matching_file`이 일치 확인과 복사를 함께 담당 —
/// 이유는 그쪽 doc comment 참고). 원본(사용자 다운로드 폴더의 파일)은 건드리지
/// 않는다 — `TempFileGuard`가 나중에 지우는 건 항상 PengPort 자신의 스크래치
/// 사본이지, 사용자가 받은 원본 파일이 아니다.
fn obtain_via_browser_assisted_download(
    recipe: &Recipe,
    archive: &ArchiveExtraction,
    app: &tauri::AppHandle,
    cancel_flag: &AtomicBool,
    tmp_dir: &Path,
) -> Result<PathBuf, String> {
    app.opener()
        .open_url(&archive.url, None::<&str>)
        .map_err(|e| format!("다운로드 페이지 열기 실패: {e}"))?;
    let _ = app.emit(
        "install:browser-download-waiting",
        serde_json::json!({
            "recipeId": recipe.id,
            "url": archive.url,
            "verification": archive.verification,
        }),
    );

    let manual_dir = manual_staging_dir(tmp_dir);
    std::fs::create_dir_all(&manual_dir)
        .map_err(|e| format!("임시 폴더 생성 실패 ({}): {e}", manual_dir.display()))?;
    let mut candidate_dirs = super::browser_download::predict_download_dirs(app);
    candidate_dirs.push(manual_dir);
    super::browser_download::watch_for_matching_file(
        &candidate_dirs,
        &archive.verification,
        cancel_flag,
        Duration::from_secs(30 * 60),
        tmp_dir,
    )
}

/// `raw_filename` 아카이브는 `dest_dir` 안에 파일 하나만 놓을 뿐 그 폴더를 소유하지
/// 않는다 — `dest_dir` 가 third-party app 이 직접 관리하는 공유 폴더(예: packwiz 가
/// 동기화하는 `.minecraft`)일 수 있다. 전체 화이트리스트 정리(`prune_disallowed_files`)를
/// 그대로 돌리면 이 archive 와 무관한 그 콘텐츠(mods/config/saves 등)까지 "선언 안 된
/// 파일"로 오인해 삭제된다. `execute_archive`(네트워크 I/O 라 직접 테스트 어려움)에서
/// 판단 로직만 분리해 순수 함수로 둔다.
fn archive_owns_dest_dir_whitelist(archive: &ArchiveExtraction) -> bool {
    archive.raw_filename.is_none()
}

/// `optional_group`도 `raw_filename`도 없는 "평범한" 압축 — 이 둘은 각자 다른 방식으로
/// `dest_dir`를 통째로 신뢰하므로(둘 다 폴더 자체가 화이트리스트라 pruning 자체를
/// skip — 그룹은 병합만, raw_filename 은 파일 하나만) [`dirty_shared_dest_dirs`]/
/// [`archive_must_run`]의 그룹화 대상이 아니다.
fn is_base_archive(archive: &ArchiveExtraction) -> bool {
    archive.optional_group.is_none() && archive.raw_filename.is_none()
}

/// `archives_with_dirty`(각 압축 + "이번에 마커가 안 맞는지") 중 base 압축이 dirty 인
/// 것들을 `dest_dir`별로 묶어 **그중 가장 작은 order**를 구한다. [`reconcile_install`]이
/// 마커 확인(I/O)을 먼저 끝내고 결과만 넘기므로 이 함수 자체는 순수 함수 — 네트워크/
/// 파일 I/O 없이 결정론적으로 테스트 가능하다.
///
/// order가 아니라 "dirty 여부"만 보고 dest_dir 전체를 재적용시키면 방향이 없어서
/// 대칭적으로 과하게 강제한다 — 낮은 order가 dirty하면 높은 order를 강제해야
/// 맞지만, 높은 order만 dirty할 땐 낮은 order는 그대로 둬도 순서 보장이 깨지지
/// 않는다(아래 [`archive_must_run`] 참고). 그래서 dest_dir당 최소 dirty order 하나만
/// 있으면 충분하다.
fn dirty_shared_dest_dirs(
    root: &Path,
    archives_with_dirty: &[(&ArchiveExtraction, bool)],
) -> HashMap<PathBuf, u32> {
    let mut min_dirty_order: HashMap<PathBuf, u32> = HashMap::new();
    for (archive, dirty) in archives_with_dirty {
        if !*dirty || !is_base_archive(archive) {
            continue;
        }
        let dest_dir = merge_dest(root, &archive.extract_to);
        min_dirty_order
            .entry(dest_dir)
            .and_modify(|min| *min = (*min).min(archive.order))
            .or_insert(archive.order);
    }
    min_dirty_order
}

/// 이 압축이 이번 실행에서 반드시 재적용돼야 하는지.
///
/// base 압축이고, 같은 dest_dir를 공유하는 다른 압축 중 **자기보다 order가 낮은**
/// dirty 항목이 있으면 자기 마커가 유효해도 무조건 재적용해야 한다 — 그 낮은
/// order가 새로 쓴 파일이 자기(높은 order)가 이겨야 할 파일을 덮어쓴 채로 남기
/// 때문. 반대로 자기보다 order가 **높은** dirty 항목만 있는 경우는 재적용할 필요가
/// 없다 — 그 높은 order가 자기 차례에 새로 쓰면 시간상 항상 나중이라 order 보장이
/// 저절로 유지된다. 그 외엔 자기 마커 유효성만 본다.
fn archive_must_run(
    archive: &ArchiveExtraction,
    dest_dir: &Path,
    dirty_dest_dirs: &HashMap<PathBuf, u32>,
    own_marker_dirty: bool,
) -> bool {
    if is_base_archive(archive) {
        if let Some(&min_dirty_order) = dirty_dest_dirs.get(dest_dir) {
            if min_dirty_order < archive.order {
                return true;
            }
        }
    }
    own_marker_dirty
}

/// 이 레시피 전용 다운로드 스크래치 폴더 — 검증 통과 전까지의 임시 파일만 여기 둔다.
/// `app_root`(레시피 전용 캐시 루트) 아래라 다른 레시피와 절대 안 겹치고, 크래시
/// 잔재는 다음 [`reconcile_install`] 시작 시 통째로 정리된다.
fn archive_tmp_dir(recipe_id: &str) -> Result<PathBuf, String> {
    Ok(super::paths::app_root(recipe_id)
        .ok_or_else(|| "app_root 미정 (%LOCALAPPDATA% 환경변수 없음)".to_string())?
        .join(".pengport-tmp"))
}

/// 압축 해제 충돌 확인 대기 중인 **검증된** 다운로드를 보관 — `archive_tmp_dir`(재조정
/// 시작할 때마다 통째로 wipe)와 별개로, 사용자가 충돌 다이얼로그에 답할 때까지
/// 살아남는다. 대용량 압축(수백MB~수GB)을 재다운로드하지 않기 위함. 파일명은
/// [`archive_content_hash`] 그대로 — 레시피가 이 압축 선언을 바꾸면 해시가 달라져
/// 자동으로 "다른 파일" 취급되므로 잘못된 재사용 위험이 없다.
fn archive_pending_conflict_dir(recipe_id: &str) -> Result<PathBuf, String> {
    Ok(super::paths::app_root(recipe_id)
        .ok_or_else(|| "app_root 미정 (%LOCALAPPDATA% 환경변수 없음)".to_string())?
        .join(".pengport-tmp-pending"))
}

fn pending_download_path(pending_dir: &Path, archive_hash: &str) -> PathBuf {
    pending_dir.join(format!("{archive_hash}.download"))
}

fn pending_resolutions_path(pending_dir: &Path, archive_hash: &str) -> PathBuf {
    pending_dir.join(format!("{archive_hash}.resolutions.json"))
}

fn write_pending_resolutions(
    pending_dir: &Path,
    archive_hash: &str,
    resolutions: &[ArchiveEntryResolution],
) -> Result<(), String> {
    std::fs::create_dir_all(pending_dir).map_err(|e| format!("대기 폴더 생성 실패: {e}"))?;
    let json = serde_json::to_vec(resolutions).map_err(|e| format!("직렬화 실패: {e}"))?;
    std::fs::write(pending_resolutions_path(pending_dir, archive_hash), json)
        .map_err(|e| format!("해결 내역 기록 실패: {e}"))
}

/// 해결 내역이 없거나 손상됐으면 `None` — 호출자가 "아직 해결 안 됨"과 동일하게
/// 취급(충돌이 다시 감지되면 다이얼로그를 또 띄움).
fn read_pending_resolutions(pending_dir: &Path, archive_hash: &str) -> Option<Vec<ArchiveEntryResolution>> {
    let bytes = std::fs::read(pending_resolutions_path(pending_dir, archive_hash)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn remove_pending_conflict_files(pending_dir: &Path, archive_hash: &str) {
    let _ = std::fs::remove_file(pending_download_path(pending_dir, archive_hash));
    let _ = std::fs::remove_file(pending_resolutions_path(pending_dir, archive_hash));
}

/// 레시피 편집으로 압축 선언이 바뀌거나 없어지면 옛 보존 다운로드가 고아로 남는다 —
/// 지금 `recipe.archives`의 어떤 해시와도 안 맞는 건 정리한다(대용량 파일이 무한정
/// 쌓이는 걸 방지). `reconcile_install` 시작 시 `.pengport-tmp` wipe와 같은 자리에서
/// 호출된다.
fn prune_orphaned_pending_conflicts(recipe: &Recipe, pending_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(pending_dir) else { return };
    let current_hashes: HashSet<String> =
        recipe.archives.iter().filter_map(|a| archive_content_hash(a).ok()).collect();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let hash = name.split('.').next().unwrap_or("");
        if !hash.is_empty() && !current_hashes.contains(hash) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// `library_install`이 실제로 적용을 시작하기 전에 부르는 pre-flight 판정 —
/// `Literal` override가 있고 아직 pending(`!is_resolved`)인 파일 중, 디스크의 지금
/// 내용이 [`write_path_fingerprint`]가 기록해둔 "마지막으로 실제로 쓴 내용"과 다른
/// 것만 충돌로 본다. 지문이 없으면(이 path를 처음 다루는 것 — 첫 설치 포함) 또는
/// 디스크에 파일 자체가 없으면(지울 것도 없음) 비교 대상이 없으니 충돌 아님.
fn detect_override_conflicts(
    recipe: &Recipe,
    selected: &HashSet<String>,
    root: &Path,
    markers_dir: &Path,
) -> Result<Vec<OverrideConflict>, String> {
    let mut conflicts = Vec::new();
    for file in effective_files(recipe, selected) {
        let Some(OverrideContent::Literal { .. }) = &file.override_content else {
            continue;
        };
        let hash = file_override_hash(file)?;
        if is_resolved(markers_dir, &hash) {
            continue;
        }
        let Some(fingerprint) = read_path_fingerprint(markers_dir, &file.path) else {
            continue;
        };
        let Ok(disk_bytes) = std::fs::read(root.join(&file.path)) else {
            continue;
        };
        if sha256_hex(&disk_bytes) != fingerprint {
            conflicts.push(OverrideConflict { path: file.path.clone() });
        }
    }
    Ok(conflicts)
}

fn execute_override(recipe_id: &str, launch: &LaunchAction, file: &RecipeFile) -> Result<(), String> {
    let Some(content) = &file.override_content else {
        return Ok(());
    };
    let root = resolve_target_root(recipe_id, launch)?;
    let markers_dir = super::paths::app_root(recipe_id)
        .ok_or_else(|| "app_root 미정 (%LOCALAPPDATA% 환경변수 없음)".to_string())?
        .join(".pengport-markers");
    match content {
        OverrideContent::Literal { content } => {
            let written = write_file_content(&root, &file.path, content)?;
            write_path_fingerprint(&markers_dir, &file.path, &written)
        }
    }
}

/// `sub` 를 `root` 기준 실제 경로로 해석. `""` 이면 루트 자체.
fn merge_dest(root: &Path, sub: &str) -> PathBuf {
    if sub.is_empty() {
        root.to_path_buf()
    } else {
        root.join(sub)
    }
}

/// 성공하면 실제로 쓴 원본 바이트를 그대로 반환 — 호출자([`execute_override`])가
/// 그걸로 적용 지문(`write_path_fingerprint`)을 남긴다(디코딩 로직을 두 번 안
/// 만들려고 여기서 계산한 bytes를 그대로 돌려줌).
fn write_file_content(root: &Path, rel_path: &str, content: &FileContent) -> Result<Vec<u8>, String> {
    let full = root.join(rel_path);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("폴더 생성 실패 ({}): {e}", parent.display()))?;
    }
    let bytes = match content {
        FileContent::Text { content } => content.as_bytes().to_vec(),
    };
    std::fs::write(&full, &bytes).map_err(|e| format!("파일 쓰기 실패 ({}): {e}", full.display()))?;
    Ok(bytes)
}

/// [`download_and_verify_to_file`]의 결과 — 정상적으로 파일을 받았는지, 아니면
/// 실제 파일이 아니라 사람이 눌러야 하는 페이지(2xx 인데 `content-type`이 HTML)를
/// 받았는지. 후자는 에러가 아니다 — 호출자가 문맥에 맞게 처리한다: 레시피 압축
/// (`execute_archive`)은 기본 브라우저로 열어 사람이 받게 하는 폴백으로,
/// third-party app 자동 다운로드(`third_party_runtime.rs`)는 그런 폴백 개념이
/// 없으니 명확한 에러로.
pub(super) enum DownloadOutcome {
    Downloaded(PathBuf),
    InteractivePage,
}

/// 응답의 `content-type` 헤더 값이 HTML(사람이 보는 페이지)인지 — 대소문자/파라미터
/// 무관(`text/html; charset=utf-8` 등). [`download_and_verify_to_file`]이 "직접
/// 받아지는 파일인지, 사람이 눌러야 하는 페이지인지" 판별하는 데 쓰는 순수 함수.
fn is_html_content_type(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|ct| ct.trim_start().to_ascii_lowercase().starts_with("text/html"))
}

/// [`download_and_verify_to_file`]가 쓰는 User-Agent — 특정 브라우저 버전과 정확히
/// 일치할 필요 없이 "그럴듯한 최신 브라우저"면 충분(대부분의 봇 차단이 이 수준으로
/// 검사함). 오래돼도 계속 동작하지만, 너무 오래된 버전은 일부 서비스가 걸러낼 수
/// 있으니 가끔 최신값으로 갱신하면 좋다.
const BROWSER_LIKE_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// 아티팩트를 청크 단위로 다운로드해 임시 파일에 스트리밍하며 SHA256 을 증분
/// 계산 → 다 받은 뒤 `verification` 과 대조(불일치 시 임시 파일 삭제 후 에러, 호출자가
/// 추출 등 후속 처리를 절대 진행하면 안 됨). 전체를 메모리에 담지 않는다 — 콘텐츠
/// 팩처럼 개별 아카이브가 실측 수 GB 인 실사용 콘텐츠가 있어 `Vec<u8>` 버퍼링은 자원
/// 고갈 위험이 있었다(`docs/design/INSTALL_PROGRESS.md`). `label` 은 에러 메시지
/// 접두어. `on_progress(downloaded_bytes, total_bytes)`는 64KB 청크마다 호출되므로
/// — 스로틀링은 호출자 책임(`Throttle`).
///
/// **"깨끗한 성공"이 아니면 전부 [`DownloadOutcome::InteractivePage`]로 취급한다**
/// — 2xx가 아닌 상태 코드(401/403/404/5xx 등 무엇이든)거나, 2xx인데 `content-type`이
/// HTML(사람이 보는 페이지)이면 그 응답을 파일로 쓰지 않고 기본 브라우저로 열어
/// 사람이 직접 받게 한다. 상태 코드만으로는 "링크 자체가 문제"인지 "자동화된
/// 요청이라 거부당했을 뿐"인지 구분할 수 없다 — 정상 공개 파일에도 자동화
/// 클라이언트에는 403을 주는 호스팅 서비스가 있다. 호스팅 서비스별로 다른 우회
/// 로직을 PengPort가 구현하지 않는다는 원칙과도 맞다.
///
/// `cancel_flag`가 있으면 매 청크마다 확인 — 켜져 있으면 즉시 [`INSTALL_CANCELLED_SENTINEL`]
/// 로 중단(레시피 설치 전용 취소 지원). third-party app 자동 다운로드처럼 취소 개념이
/// 없는 호출자는 `None`을 넘긴다.
pub(super) fn download_and_verify_to_file(
    url: &str,
    verification: &ArtifactVerification,
    label: &str,
    timeout: Duration,
    tmp_dir: &Path,
    cancel_flag: Option<&AtomicBool>,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> Result<DownloadOutcome, String> {
    use std::io::{Read, Write};

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        // ureq 기본값(true)은 4xx/5xx를 `Err(Error::StatusCode)`로 바꿔버려 아래
        // `status.is_success()` 체크가 무의미해진다 — `Ok(response)`로 받아서 직접
        // 상태코드를 보고 판단하게 끈다.
        .http_status_as_error(false)
        // ureq 기본 User-Agent(`ureq/x.y.z`)로는 다수 호스팅 서비스가 봇으로 간주해
        // 거부할 수 있다 — 브라우저처럼 보이게 해서 불필요한 브라우저 폴백을 줄인다.
        // 이걸로 못 뚫는 서비스가 있어도 해는 없다(아래에서 어차피 브라우저로 넘어갈
        // 뿐).
        .user_agent(BROWSER_LIKE_USER_AGENT)
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| format!("{label} 다운로드 실패: {e}"))?;
    let status = response.status();
    if !status.is_success()
        || is_html_content_type(response.headers().get("content-type").and_then(|v| v.to_str().ok()))
    {
        return Ok(DownloadOutcome::InteractivePage);
    }
    let total_bytes = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());

    std::fs::create_dir_all(tmp_dir)
        .map_err(|e| format!("임시 폴더 생성 실패 ({}): {e}", tmp_dir.display()))?;
    let tmp_path = tmp_dir.join("download.part");
    let mut file = std::fs::File::create(&tmp_path)
        .map_err(|e| format!("{label} 임시 파일 생성 실패 ({}): {e}", tmp_path.display()))?;

    let mut hasher = Sha256Verifier::new();
    let mut reader = response.body_mut().as_reader();
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    loop {
        if let Some(flag) = cancel_flag {
            check_cancelled(flag)?;
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("{label} 응답 읽기 실패: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("{label} 임시 파일 쓰기 실패 ({}): {e}", tmp_path.display()))?;
        hasher.update(&buf[..n]);
        downloaded += n as u64;
        on_progress(downloaded, total_bytes);
    }
    drop(file);

    if let Err(e) = hasher.finish(verification) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!("{label} 검증 실패 — 설치 중단: {e}"));
    }
    Ok(DownloadOutcome::Downloaded(tmp_path))
}

/// Windows의 "인터넷에서 받음" 표식(Zone.Identifier NTFS ADS)을 남긴다. 브라우저로
/// 같은 파일을 받았으면 Windows Defender SmartScreen이 실행 시 경고를 띄우는데,
/// PengPort는 자체 HTTP 클라이언트로 받아 이 표식 없이 그대로 쓰기 때문에 그 경고가
/// 조용히 사라진다 — 레시피 자체가 정직해도 그 안의 실행 파일이 악성인 경우에 대한
/// 마지막 OS 방어선이라 자동화 과정에서도 복원한다. 압축 추출은 PengPort가 엔트리를
/// 직접 풀어 쓰므로(탐색기의 "부모 zip 표식이 자식까지 전파" 동작을 안 탐) 다운로드
/// 원본뿐 아니라 실제로 디스크에 남는 각 파일에 개별 적용해야 한다. ADS 미지원
/// 파일시스템 등에서 실패해도 설치 자체를 막을 이유는 없어 로그만 남기고 넘어간다.
#[cfg(windows)]
fn stamp_mark_of_the_web(path: &Path) {
    let ads_path = format!("{}:Zone.Identifier", path.display());
    if let Err(e) = std::fs::write(&ads_path, b"[ZoneTransfer]\r\nZoneId=3\r\n") {
        eprintln!("Mark of the Web 기록 실패 ({}): {e}", path.display());
    }
}

#[cfg(not(windows))]
fn stamp_mark_of_the_web(_path: &Path) {}

/// 임시 파일(스트리밍 다운로드 결과)을 매직 바이트로 형식 판별해([`sniff_archive_kind`])
/// `dest_dir` 에 추출. `dest_dir` 는 비우지 않고 병합 — 여러 압축이 같은 루트/하위
/// 폴더에 겹쳐 설치되는 걸 지원(예: 본체 + 추가 콘텐츠 팩). 겹쳐 쓴 뒤 화이트리스트
/// 정리(`prune_disallowed_files`)가 뒤따른다.
///
/// `strip_root`: 그룹 전용 콘텐츠 압축(예: 콘텐츠 팩 하나가 `Content/` 폴더 하나로만
/// 구성)은 그 최상위 폴더 자체를 벗겨내고 그 안의 내용을 바로 `dest_dir`(=owned 리프
/// 경로, 예: `SampleApp/Content`)에 푼다 — 레시피의 `extract_to`가 이미 최종 owned 경로를
/// 정확히 가리키게 하기 위함(`execute_archive`의 "통째 신뢰" 전제 — dest_dir 자체가
/// 이 압축의 배타적 소유 폴더여야 안전하게 통째로 지우고 다시 풀 수 있다).
///
/// 압축 라이브러리의 일괄 추출 헬퍼(`ZipArchive::extract`/`sevenz_rust2::decompress`)
/// 대신 엔트리를 하나씩 순회한다 — 콘텐츠 팩처럼 파일이 1000개 넘는 압축에서 "지금
/// 몇 번째 푸는 중"을 알 수 있는 유일한 방법이라(`docs/design/INSTALL_PROGRESS.md`).
///
/// `raw_filename`이 있으면 압축으로 취급하지 않고, 검증된 임시 파일을
/// `dest_dir/raw_filename`에 그대로 배치한다 — 아이콘/실행파일/jar 같은 단일 파일
/// 자산용(`ArchiveExtraction::raw_filename` 참고).
/// [`extract_archive_file`]의 부가 정보를 묶은 컨텍스트 — 매개변수 개수를 줄이려고
/// 묶었을 뿐 별도 의미는 없다. `resolutions`도 여기 포함 — 압축 해제 충돌을 사용자가
/// 어떻게 해결했는지(빈 맵이면 "충돌 없었음"과 동일).
struct ExtractProgressContext<'a> {
    app: &'a tauri::AppHandle,
    recipe_id: &'a str,
    resolutions: &'a HashMap<String, ArchiveEntryResolution>,
}

fn extract_archive_file(
    tmp_path: &Path,
    url: &str,
    dest_dir: &Path,
    strip_root: bool,
    raw_filename: Option<&str>,
    ctx: ExtractProgressContext,
    cancel_flag: Option<&AtomicBool>,
) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("설치 대상 폴더 생성 실패 ({}): {e}", dest_dir.display()))?;

    if let Some(filename) = raw_filename {
        if matches!(ctx.resolutions.get(filename), Some(ArchiveEntryResolution::Skip { .. })) {
            return Ok(vec![]);
        }
        let dst = match ctx.resolutions.get(filename) {
            Some(ArchiveEntryResolution::Rename { .. }) => unique_fs_path(dest_dir, filename),
            _ => dest_dir.join(filename),
        };
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("설치 대상 폴더 생성 실패 ({}): {e}", parent.display()))?;
        }
        std::fs::copy(tmp_path, &dst)
            .map_err(|e| format!("파일 배치 실패 ({}): {e}", dst.display()))?;
        stamp_mark_of_the_web(&dst);
        return Ok(vec![relative_manifest_path(dest_dir, &dst)]);
    }

    let throttle = Throttle::new(Duration::from_millis(150));
    let on_progress = |extracted: usize, total: usize| {
        if throttle.allow() || extracted == total {
            let _ = ctx.app.emit(
                "install:extract-progress",
                serde_json::json!({
                    "recipeId": ctx.recipe_id, "label": url,
                    "extractedEntries": extracted, "totalEntries": total
                }),
            );
        }
    };

    match sniff_archive_kind(tmp_path)? {
        ArchiveKind::Zip => {
            let file = std::fs::File::open(tmp_path)
                .map_err(|e| format!("임시 파일 열기 실패 ({}): {e}", tmp_path.display()))?;
            let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("zip 열기 실패: {e}"))?;
            extract_zip_with_progress(&mut archive, dest_dir, strip_root, cancel_flag, ctx.resolutions, on_progress)
        }
        ArchiveKind::SevenZ => {
            let file = std::fs::File::open(tmp_path)
                .map_err(|e| format!("임시 파일 열기 실패 ({}): {e}", tmp_path.display()))?;
            extract_7z_with_progress(file, dest_dir, strip_root, cancel_flag, ctx.resolutions, on_progress)
        }
    }
}

/// 추출된 전체 경로를 `dest_dir` 기준 상대경로 문자열(`/` 구분자)로 — 그룹 압축의
/// 매니페스트([`execute_archive`]가 반환, [`remove_grouped_archive_content`]가 소비)에
/// 기록하는 형식과 `prune_disallowed_files`의 화이트리스트 경로 형식을 통일한다.
fn relative_manifest_path(dest_dir: &Path, full_path: &Path) -> String {
    full_path
        .strip_prefix(dest_dir)
        .unwrap_or(full_path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 압축 안 엔트리 하나의 스캔 결과 — 실제로 쓰기 전에 목적지 경로 + 원본 CRC32만
/// 안다(압축을 풀지 않고 zip/7z 헤더에서 바로 읽음). `rel_path`는
/// [`relative_manifest_path`]와 같은 형식이라 [`ArchiveEntryResolution`]의 `path`와
/// 그대로 대조된다.
struct ScannedEntry {
    rel_path: String,
    dest_path: PathBuf,
    crc32: u32,
}

/// [`execute_archive`]가 실제로 쓰기 전에 부르는 스캔 — 디렉토리 엔트리, CRC를 못 구하는
/// 항목(일부 7z 케이스)은 대상에서 빠진다(그런 항목은 충돌 판정 없이 예전처럼 그냥 씀).
fn scan_archive_entries(
    tmp_path: &Path,
    dest_dir: &Path,
    strip_root: bool,
    raw_filename: Option<&str>,
) -> Result<Vec<ScannedEntry>, String> {
    if let Some(filename) = raw_filename {
        let crc32 = crc32_of_file(tmp_path)?;
        return Ok(vec![ScannedEntry {
            rel_path: filename.to_string(),
            dest_path: dest_dir.join(filename),
            crc32,
        }]);
    }

    match sniff_archive_kind(tmp_path)? {
        ArchiveKind::Zip => {
            let file = std::fs::File::open(tmp_path)
                .map_err(|e| format!("임시 파일 열기 실패 ({}): {e}", tmp_path.display()))?;
            let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("zip 열기 실패: {e}"))?;
            let mut out = Vec::new();
            for i in 0..archive.len() {
                let entry = archive.by_index(i).map_err(|e| format!("zip 항목 열기 실패: {e}"))?;
                if entry.is_dir() {
                    continue;
                }
                let name = entry.name().to_string();
                let crc32 = entry.crc32();
                if let Some(dest_path) = safe_join_archive_entry(dest_dir, &name, strip_root)? {
                    out.push(ScannedEntry { rel_path: relative_manifest_path(dest_dir, &dest_path), dest_path, crc32 });
                }
            }
            Ok(out)
        }
        ArchiveKind::SevenZ => {
            let file = std::fs::File::open(tmp_path)
                .map_err(|e| format!("임시 파일 열기 실패 ({}): {e}", tmp_path.display()))?;
            let reader = sevenz_rust2::ArchiveReader::new(file, sevenz_rust2::Password::empty())
                .map_err(|e| format!("7z 열기 실패: {e}"))?;
            let mut out = Vec::new();
            for entry in &reader.archive().files {
                if entry.is_directory() || !entry.has_crc {
                    continue;
                }
                if let Some(dest_path) = safe_join_archive_entry(dest_dir, entry.name(), strip_root)? {
                    out.push(ScannedEntry {
                        rel_path: relative_manifest_path(dest_dir, &dest_path),
                        dest_path,
                        crc32: entry.crc as u32,
                    });
                }
            }
            Ok(out)
        }
    }
}

/// [`scan_archive_entries`]가 계산한 `dest_path`는 "자연스러운"(압축 안 경로 그대로)
/// 위치일 뿐 — `archive.path_overrides`에 의한 재배치는 실제 추출 후 별도 단계인
/// [`apply_path_overrides`]가 한다. 충돌 판정([`detect_archive_conflicts`])은 파일이
/// **실제로 최종 놓일 위치**를 봐야 하므로, 스캔 직후 같은 재배치 규칙
/// ([`resolve_path_override_target`] — 실제 재배치가 쓰는 것과 동일한 순수 함수)을
/// 미리 적용해 `dest_path`만 보정한다. `rel_path`는 일부러 안 건드린다 — 실제 추출
/// 시점의 충돌 해결(`extract_zip_with_progress` 등)이 여전히 자연스러운 경로를
/// 키로 조회하므로, 여기서 같이 바꾸면 사용자가 고른 해결책이 적용 시점에 못 찾아진다.
fn apply_path_overrides_to_scan(scanned: Vec<ScannedEntry>, root: &Path, overrides: &[PathOverride]) -> Vec<ScannedEntry> {
    if overrides.is_empty() {
        return scanned;
    }
    scanned
        .into_iter()
        .map(|mut entry| {
            if let Some(new_to) = resolve_path_override_target(&entry.rel_path, overrides) {
                entry.dest_path = root.join(&new_to);
            }
            entry
        })
        .collect()
}

fn crc32_of_file(path: &Path) -> Result<u32, String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| format!("파일 열기 실패 ({}): {e}", path.display()))?;
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| format!("파일 읽기 실패 ({}): {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize())
}

/// 스캔된 엔트리 중 "전체 허용 + `ask_on_conflict`" 폴더 밑에 있고, 디스크에 이미
/// 다른 내용의 파일이 있는 것만 충돌로 본다. 내용이 완전히 같으면(CRC 일치) 잃을 게
/// 없으니 충돌 아님 — 조용히 덮어써도 됨.
fn detect_archive_conflicts(scanned: &[ScannedEntry], recipe: &Recipe, root: &Path) -> Vec<String> {
    let ask_folders: Vec<PathBuf> = recipe
        .folder_rules
        .iter()
        .filter(|r| matches!(r.mode, FolderRuleMode::Passthrough { ask_on_conflict: true }))
        .map(|r| root.join(&r.path))
        .collect();
    if ask_folders.is_empty() {
        return Vec::new();
    }
    scanned
        .iter()
        .filter(|e| ask_folders.iter().any(|f| e.dest_path.starts_with(f)))
        .filter(|e| match std::fs::metadata(&e.dest_path) {
            Ok(m) if m.is_file() => crc32_of_file(&e.dest_path).map(|c| c != e.crc32).unwrap_or(false),
            _ => false,
        })
        .map(|e| e.rel_path.clone())
        .collect()
}

/// zip 엔트리를 하나씩 순회하며 추출 — 진행률 콜백이 필요한 이유는
/// [`extract_archive_file`] 참고. 경로 안전 검사는 [`safe_join_archive_entry`] 하나로
/// strip_root 유무 양쪽을 처리(zip 크레이트 자체의 `enclosed_name()`이 하던 zip-slip
/// 방어를 7z 쪽과 같은 규칙으로 통일). `resolutions`에 없는 항목(=충돌 없었음)은 예전
/// 그대로 조용히 덮어씀. 반환값 = 실제로 쓴 파일들의 `dest_dir` 기준 상대경로 목록
/// (디렉토리 엔트리 제외) — 그룹 압축의 매니페스트 기록용.
fn extract_zip_with_progress<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    dest_dir: &Path,
    strip_root: bool,
    cancel_flag: Option<&AtomicBool>,
    resolutions: &HashMap<String, ArchiveEntryResolution>,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<Vec<String>, String> {
    let total = archive.len();
    let mut written = Vec::new();
    for i in 0..total {
        if let Some(flag) = cancel_flag {
            check_cancelled(flag)?;
        }
        let mut entry = archive.by_index(i).map_err(|e| format!("zip 항목 열기 실패: {e}"))?;
        let name = entry.name().to_string();
        let is_dir = entry.is_dir();
        let natural_path = match safe_join_archive_entry(dest_dir, &name, strip_root)? {
            Some(p) => p,
            None => {
                on_progress(i + 1, total);
                continue;
            }
        };
        if is_dir {
            std::fs::create_dir_all(&natural_path)
                .map_err(|e| format!("폴더 생성 실패 ({}): {e}", natural_path.display()))?;
        } else {
            let rel_path = relative_manifest_path(dest_dir, &natural_path);
            if let Some(ArchiveEntryResolution::Skip { .. }) = resolutions.get(&rel_path) {
                on_progress(i + 1, total);
                continue;
            }
            let out_path = match resolutions.get(&rel_path) {
                Some(ArchiveEntryResolution::Rename { .. }) => unique_fs_path(
                    natural_path.parent().unwrap_or(dest_dir),
                    natural_path.file_name().and_then(|n| n.to_str()).unwrap_or(&name),
                ),
                _ => natural_path,
            };
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("폴더 생성 실패 ({}): {e}", parent.display()))?;
            }
            let mut out_file = std::fs::File::create(&out_path)
                .map_err(|e| format!("파일 생성 실패 ({}): {e}", out_path.display()))?;
            std::io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("zip 항목 쓰기 실패 ({}): {e}", out_path.display()))?;
            drop(out_file);
            stamp_mark_of_the_web(&out_path);
            written.push(relative_manifest_path(dest_dir, &out_path));
        }
        on_progress(i + 1, total);
    }
    Ok(written)
}

/// 7z 엔트리를 하나씩 순회하며 추출 — `archive().files.len()`으로 전체 개수를 먼저
/// 얻는다(헤더 파싱만, 엔트리 데이터는 아직 안 읽음). 경로 안전 검사는 zip 쪽과 같은
/// [`safe_join_archive_entry`] 공유. `resolutions` 처리는 [`extract_zip_with_progress`]와
/// 동일 규칙. 반환값도 동일.
fn extract_7z_with_progress(
    file: std::fs::File,
    dest_dir: &Path,
    strip_root: bool,
    cancel_flag: Option<&AtomicBool>,
    resolutions: &HashMap<String, ArchiveEntryResolution>,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<Vec<String>, String> {
    let mut reader = sevenz_rust2::ArchiveReader::new(file, sevenz_rust2::Password::empty())
        .map_err(|e| format!("7z 열기 실패: {e}"))?;
    let total = reader.archive().files.len();
    let mut done = 0usize;
    let mut written: Vec<String> = Vec::new();
    reader
        .for_each_entries(|entry, r| {
            done += 1;
            if let Some(flag) = cancel_flag {
                if let Err(msg) = check_cancelled(flag) {
                    return Err(sevenz_rust2::Error::Other(msg.into()));
                }
            }
            let outcome = match safe_join_archive_entry(dest_dir, entry.name(), strip_root) {
                Ok(Some(natural_path)) => {
                    let rel_path = relative_manifest_path(dest_dir, &natural_path);
                    if !entry.is_directory()
                        && matches!(resolutions.get(&rel_path), Some(ArchiveEntryResolution::Skip { .. }))
                    {
                        Ok(true)
                    } else {
                        let path = if !entry.is_directory()
                            && matches!(resolutions.get(&rel_path), Some(ArchiveEntryResolution::Rename { .. }))
                        {
                            unique_fs_path(
                                natural_path.parent().unwrap_or(dest_dir),
                                natural_path.file_name().and_then(|n| n.to_str()).unwrap_or(entry.name()),
                            )
                        } else {
                            natural_path
                        };
                        let result = sevenz_rust2::default_entry_extract_fn(entry, r, &path);
                        if result.is_ok() && !entry.is_directory() {
                            stamp_mark_of_the_web(&path);
                            written.push(relative_manifest_path(dest_dir, &path));
                        }
                        result
                    }
                }
                Ok(None) => Ok(true),
                Err(msg) => Err(sevenz_rust2::Error::Other(msg.into())),
            };
            on_progress(done, total);
            outcome
        })
        .map_err(|e| format!("7z 추출 실패: {e}"))?;
    Ok(written)
}

/// `dir` 안에 `filename`이 이미 있으면 "이름 (2).ext" 형태로 충돌을 피한다(확장자
/// 보존, 실제로 그 이름의 파일이 없을 때까지 증가) — 프론트 `file-tree-picker.tsx`의
/// `uniqueTreePath`와 같은 규칙을 그대로 파일시스템에 적용한 버전. 압축 해제 충돌
/// 해결에서 "이름 바꿔 복사"가 쓴다.
fn unique_fs_path(dir: &Path, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let dot = filename.rfind('.');
    let (stem, ext) = match dot {
        Some(i) if i > 0 => (&filename[..i], &filename[i..]),
        _ => (filename, ""),
    };
    let mut i = 2;
    loop {
        let candidate = dir.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
        i += 1;
    }
}

/// 압축 안 항목 이름(`entry_name`, 예: `"GroupA/track1.dat"`)을 `dest`에 안전하게
/// 조인 — zip/7z 공용, `..`/루트/드라이브 컴포넌트를 거부해 zip-slip을 막는다(zip
/// 크레이트의 `enclosed_name()`, `sevenz_rust2`의 비공개 `safe_join`과 같은 규칙).
///
/// `strip_first`가 true 면 최상위 컴포넌트 하나(그룹 전용 콘텐츠 압축의 owned 폴더
/// 자체, 예: `"GroupA"`)를 먼저 벗겨낸다 — 그 컴포넌트 자체를 가리키는 항목(디렉토리
/// 엔트리 `"GroupA"`, 또는 그 안에 하위 경로가 하나도 없는 경우)은 `Ok(None)`으로
/// 건너뛴다(`dest`는 이미 [`execute_archive`]가 만들어둠). `strip_first`가 false 면
/// 전체 경로를 그대로 조인한다.
fn safe_join_archive_entry(dest: &Path, entry_name: &str, strip_first: bool) -> Result<Option<PathBuf>, String> {
    use std::path::Component;

    let normalized = entry_name.replace('\\', "/");
    let mut components = Path::new(&normalized).components();
    if strip_first {
        match components.next() {
            Some(Component::Normal(_)) => {}
            None => return Ok(None),
            Some(_) => return Err(format!("unsafe entry path: {entry_name}")),
        }
    }

    let mut result = dest.to_path_buf();
    let mut any = false;
    for component in components {
        match component {
            Component::Normal(part) => {
                result.push(part);
                any = true;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("unsafe entry path escapes destination: {entry_name}"));
            }
        }
    }
    if strip_first && !any {
        Ok(None)
    } else {
        Ok(Some(result))
    }
}

/// 압축 해제 직후, `dest_dir` 안에 실제로 존재하는 모든 파일을 `allowed`(레시피
/// `files` 화이트리스트, `extract_to` 기준 상대경로)와 대조해서 선언 안 된 파일을
/// 즉시 삭제한다 — `exclude`에 해당하는 하위트리(그룹 전용 콘텐츠 압축이 통째로
/// 소유하는 폴더, `Recipe.folder_rules`가 선언한 모든 폴더 — [`folder_rule_dest_dirs`]
/// 참고, 그쪽은 [`apply_folder_rules`]의 전속 관할이라 여기서 손대면 안 됨)는 대상에서
/// 제외. 다운로드 링크가 바뀌어 다른 미러를 쓰게 되거나(원치 않는 파일이 섞여 들어올
/// 수 있음), 레시피 편집으로 화이트리스트가 줄어든 경우(예전 버전이 남긴 파일) 둘 다
/// 이걸로 잡힌다. zip-slip 방어 자체는 `zip`(`enclosed_name`)/`sevenz_rust2`(`safe_join`)
/// 양쪽 크레이트가 이미 담당하므로, 이 단계는 순수 콘텐츠 위생 관리.
fn prune_disallowed_files(
    dest_dir: &Path,
    extract_to: &str,
    allowed: &HashSet<String>,
    exclude: &[PathBuf],
) -> Result<(), String> {
    let mut files = Vec::new();
    collect_leaf_paths(dest_dir, exclude, &mut files)?;

    let extract_to_path = Path::new(extract_to);
    for file_path in files {
        let rel_from_dest = file_path
            .strip_prefix(dest_dir)
            .map_err(|e| format!("경로 계산 실패 ({}): {e}", file_path.display()))?;
        let full_rel = if extract_to.is_empty() {
            rel_from_dest.to_path_buf()
        } else {
            extract_to_path.join(rel_from_dest)
        };
        let full_rel_str = full_rel.to_string_lossy().replace('\\', "/");
        if !allowed.contains(&full_rel_str) {
            std::fs::remove_file(&file_path)
                .map_err(|e| format!("화이트리스트에 없는 파일 삭제 실패 ({}): {e}", file_path.display()))?;
        }
    }
    prune_empty_subdirs(dest_dir, exclude)
}

/// `dir` 아래 재귀적으로, 디렉토리가 아닌 항목(파일 + 심볼릭 링크) 전부의 경로를 모은다.
/// `exclude`에 정확히 일치하는 하위 디렉토리는 안으로 내려가지 않는다(그룹 전용
/// 콘텐츠 폴더 보호 — [`prune_disallowed_files`] 참고).
fn collect_leaf_paths(dir: &Path, exclude: &[PathBuf], out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("디렉토리 읽기 실패 ({}): {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("디렉토리 항목 읽기 실패 ({}): {e}", dir.display()))?;
        let path = entry.path();
        if exclude.iter().any(|e| e == &path) {
            continue;
        }
        let is_dir = entry
            .file_type()
            .map_err(|e| format!("파일 타입 확인 실패 ({}): {e}", path.display()))?
            .is_dir();
        if is_dir {
            collect_leaf_paths(&path, exclude, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

/// 파일 삭제 후 남은 빈 디렉토리를 하위부터 정리. `dir` 자기 자신은 지우지 않는다
/// (target 루트는 항상 남아있어야 함 — 다음 오버라이드 스텝이 그 위에 씀). `exclude`는
/// [`collect_leaf_paths`]와 동일 — 그룹 전용 콘텐츠 폴더는 손대지 않는다.
fn prune_empty_subdirs(dir: &Path, exclude: &[PathBuf]) -> Result<(), String> {
    let subdirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("디렉토리 읽기 실패 ({}): {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| !exclude.iter().any(|e| e == p))
        .collect();
    for sub in subdirs {
        prune_empty_subdirs(&sub, exclude)?;
        let is_empty = std::fs::read_dir(&sub)
            .map_err(|e| format!("디렉토리 읽기 실패 ({}): {e}", sub.display()))?
            .next()
            .is_none();
        if is_empty {
            std::fs::remove_dir(&sub)
                .map_err(|e| format!("빈 폴더 정리 실패 ({}): {e}", sub.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use pengport_shared::library::FolderRule;

    use super::*;

    // --- stamp_mark_of_the_web ---

    #[test]
    #[cfg(windows)]
    fn stamp_mark_of_the_web_writes_zone_identifier_stream() {
        let dir = temp_test_dir("motw");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("downloaded.exe");
        std::fs::write(&path, b"fake exe bytes").unwrap();

        stamp_mark_of_the_web(&path);

        let ads = std::fs::read_to_string(format!("{}:Zone.Identifier", path.display())).unwrap();
        assert!(ads.contains("ZoneId=3"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exe_under_root_true_when_exe_inside_root() {
        let root = Path::new("/apps/sampleapp");
        let exe = Path::new("/apps/sampleapp/SampleApp.exe");
        assert!(exe_under_root(Some(exe), root));
    }

    #[test]
    fn exe_under_root_true_for_nested_subdir() {
        let root = Path::new("/apps/sampleapp");
        let exe = Path::new("/apps/sampleapp/bin/updater.exe");
        assert!(exe_under_root(Some(exe), root));
    }

    #[test]
    fn exe_under_root_false_when_outside_root() {
        let root = Path::new("/apps/sampleapp");
        let exe = Path::new("/apps/other-app/other.exe");
        assert!(!exe_under_root(Some(exe), root));
    }

    #[test]
    fn exe_under_root_false_when_exe_missing() {
        let root = Path::new("/apps/sampleapp");
        assert!(!exe_under_root(None, root));
    }

    #[test]
    fn exe_under_root_false_for_sibling_with_shared_prefix() {
        // "/apps/sampleapp2"는 "/apps/sampleapp"으로 시작하지 않는 별개 경로 —
        // 문자열 prefix가 아니라 경로 컴포넌트 단위 비교(Path::starts_with)여야 함.
        let root = Path::new("/apps/sampleapp");
        let exe = Path::new("/apps/sampleapp2/other.exe");
        assert!(!exe_under_root(Some(exe), root));
    }

    #[test]
    fn safe_join_no_strip_joins_full_relative_path() {
        let dest = Path::new("/dest");
        let result = safe_join_archive_entry(dest, "sub/dir/file.txt", false).unwrap();
        assert_eq!(result, Some(dest.join("sub").join("dir").join("file.txt")));
    }

    #[test]
    fn safe_join_strip_first_removes_top_level_component() {
        let dest = Path::new("/dest");
        // "GroupA/track1.dat" → 최상위 "GroupA" 벗겨내고 dest 밑에 "track1.dat" 만.
        let result = safe_join_archive_entry(dest, "GroupA/track1.dat", true).unwrap();
        assert_eq!(result, Some(dest.join("track1.dat")));
    }

    #[test]
    fn safe_join_strip_first_skips_root_only_entry() {
        // 최상위 폴더 자체를 가리키는 디렉토리 엔트리("GroupA")는 dest 가 이미 있으니
        // 건너뛰어야 한다(execute_archive 가 dest_dir 를 미리 만들어둠).
        let dest = Path::new("/dest");
        assert_eq!(safe_join_archive_entry(dest, "GroupA", true).unwrap(), None);
        assert_eq!(safe_join_archive_entry(dest, "GroupA/", true).unwrap(), None);
    }

    #[test]
    fn safe_join_rejects_parent_dir_escape() {
        let dest = Path::new("/dest");
        assert!(safe_join_archive_entry(dest, "../../etc/passwd", false).is_err());
        assert!(safe_join_archive_entry(dest, "GroupA/../../../etc/passwd", true).is_err());
    }

    #[test]
    fn safe_join_rejects_absolute_and_backslash_paths() {
        let dest = Path::new("/dest");
        // zip 은 항상 "/" 를 쓰지만, 압축기가 Windows 스타일 "\" 로 만든 항목도
        // 정규화 후 같은 규칙(루트/드라이브 컴포넌트 거부)으로 걸러야 한다.
        assert!(safe_join_archive_entry(dest, "/etc/passwd", false).is_err());
        assert!(safe_join_archive_entry(dest, "..\\..\\windows\\system32", false).is_err());
    }

    fn sample_archive_with(raw_filename: Option<&str>, extract_to: &str) -> ArchiveExtraction {
        ArchiveExtraction {
            url: "https://cdn.example.com/tool.jar".to_string(),
            label: None,
            verification: ArtifactVerification::Sha256 { hash: "0".repeat(64) },
            order: 0,
            extract_to: extract_to.to_string(),
            optional_group: None,
            raw_filename: raw_filename.map(str::to_string),
            path_overrides: Vec::new(),
        }
    }

    #[test]
    fn raw_filename_archive_does_not_own_dest_dir_whitelist() {
        let archive = sample_archive_with(Some("bootstrap.jar"), ".minecraft");
        assert!(!archive_owns_dest_dir_whitelist(&archive));
    }

    #[test]
    fn normal_archive_owns_dest_dir_whitelist() {
        let archive = sample_archive_with(None, "SampleApp/Content");
        assert!(archive_owns_dest_dir_whitelist(&archive));
    }

    #[test]
    fn apply_path_overrides_moves_matched_file_to_declared_destination() {
        let dest_dir = temp_test_dir("path-overrides-move");
        std::fs::create_dir_all(&dest_dir).unwrap();
        std::fs::write(dest_dir.join("asset1.dat"), b"skin content").unwrap();
        let written = vec!["asset1.dat".to_string()];
        let overrides = vec![PathOverride {
            from: "asset1.dat".to_string(),
            to: "SampleApp/Image/asset1.dat".to_string(),
        }];

        let result = apply_path_overrides(&written, &dest_dir, &dest_dir, &overrides).unwrap();

        assert_eq!(result, vec!["SampleApp/Image/asset1.dat".to_string()]);
        assert!(!dest_dir.join("asset1.dat").exists());
        assert_eq!(
            std::fs::read(dest_dir.join("SampleApp/Image/asset1.dat")).unwrap(),
            b"skin content"
        );
        let _ = std::fs::remove_dir_all(&dest_dir);
    }

    #[test]
    fn apply_path_overrides_leaves_unmatched_files_untouched() {
        let dest_dir = temp_test_dir("path-overrides-unmatched");
        std::fs::create_dir_all(&dest_dir).unwrap();
        std::fs::write(dest_dir.join("other.txt"), b"x").unwrap();
        let written = vec!["other.txt".to_string()];
        let overrides = vec![PathOverride {
            from: "asset1.dat".to_string(),
            to: "SampleApp/Image/asset1.dat".to_string(),
        }];

        let result = apply_path_overrides(&written, &dest_dir, &dest_dir, &overrides).unwrap();

        assert_eq!(result, vec!["other.txt".to_string()]);
        assert!(dest_dir.join("other.txt").exists());
        let _ = std::fs::remove_dir_all(&dest_dir);
    }

    #[test]
    fn apply_path_overrides_no_op_when_empty() {
        let written = vec!["a.txt".to_string(), "b.txt".to_string()];
        let dummy = PathBuf::from("/nonexistent");
        let result = apply_path_overrides(&written, &dummy, &dummy, &[]).unwrap();
        assert_eq!(result, written);
    }

    #[test]
    fn apply_path_overrides_moves_whole_folder_via_prefix_match() {
        // "A/"(트레일링 슬래시) — rsync `src/` 관례처럼 내용만 옮기고 "A" 폴더 이름은
        // 사라짐.
        let dest_dir = temp_test_dir("path-overrides-folder-prefix");
        std::fs::create_dir_all(dest_dir.join("A/sub")).unwrap();
        std::fs::write(dest_dir.join("A/installer.exe"), b"exe").unwrap();
        std::fs::write(dest_dir.join("A/sub/x.txt"), b"x").unwrap();
        let written = vec!["A/installer.exe".to_string(), "A/sub/x.txt".to_string()];
        let overrides = vec![PathOverride { from: "A/".to_string(), to: "".to_string() }];

        let mut result = apply_path_overrides(&written, &dest_dir, &dest_dir, &overrides).unwrap();
        result.sort();

        assert_eq!(result, vec!["installer.exe".to_string(), "sub/x.txt".to_string()]);
        assert!(dest_dir.join("installer.exe").exists());
        assert!(dest_dir.join("sub/x.txt").exists());
        // 이제 빈 "A" 폴더 자체는 이 함수 책임이 아니다 — 실제 파이프라인에서는 바로
        // 다음에 도는 prune_disallowed_files(끝에 prune_empty_subdirs 포함)가 지운다.
        let _ = std::fs::remove_dir_all(&dest_dir);
    }

    #[test]
    fn apply_path_overrides_keeps_folder_name_without_trailing_slash() {
        // "A"(슬래시 없음) — rsync `src` 관례처럼 폴더 이름을 유지한 채 `to` 밑에
        // 통째로 얹는다(`Moved/A/...`).
        let dest_dir = temp_test_dir("path-overrides-folder-keep-name");
        std::fs::create_dir_all(dest_dir.join("A/sub")).unwrap();
        std::fs::write(dest_dir.join("A/installer.exe"), b"exe").unwrap();
        std::fs::write(dest_dir.join("A/sub/x.txt"), b"x").unwrap();
        let written = vec!["A/installer.exe".to_string(), "A/sub/x.txt".to_string()];
        let overrides = vec![PathOverride { from: "A".to_string(), to: "Moved".to_string() }];

        let mut result = apply_path_overrides(&written, &dest_dir, &dest_dir, &overrides).unwrap();
        result.sort();

        assert_eq!(result, vec!["Moved/A/installer.exe".to_string(), "Moved/A/sub/x.txt".to_string()]);
        assert!(dest_dir.join("Moved/A/installer.exe").exists());
        assert!(dest_dir.join("Moved/A/sub/x.txt").exists());
        let _ = std::fs::remove_dir_all(&dest_dir);
    }

    #[test]
    fn apply_path_overrides_exact_match_wins_over_folder_prefix() {
        let dest_dir = temp_test_dir("path-overrides-exact-wins");
        std::fs::create_dir_all(dest_dir.join("A")).unwrap();
        std::fs::write(dest_dir.join("A/keep_here.txt"), b"special").unwrap();
        std::fs::write(dest_dir.join("A/normal.txt"), b"normal").unwrap();
        let written = vec!["A/keep_here.txt".to_string(), "A/normal.txt".to_string()];
        // 폴더 규칙(A/ -> 루트)보다 먼저 선언돼 있어도, 정확히 일치하는 예외가 이겨야 한다.
        let overrides = vec![
            PathOverride { from: "A/keep_here.txt".to_string(), to: "Special/keep_here.txt".to_string() },
            PathOverride { from: "A/".to_string(), to: "".to_string() },
        ];

        let mut result = apply_path_overrides(&written, &dest_dir, &dest_dir, &overrides).unwrap();
        result.sort();

        assert_eq!(result, vec!["Special/keep_here.txt".to_string(), "normal.txt".to_string()]);
        let _ = std::fs::remove_dir_all(&dest_dir);
    }

    #[test]
    fn is_html_content_type_matches_html_regardless_of_case_or_charset_param() {
        assert!(is_html_content_type(Some("text/html; charset=utf-8")));
        assert!(is_html_content_type(Some("TEXT/HTML")));
        assert!(!is_html_content_type(Some("application/zip")));
        assert!(!is_html_content_type(Some("application/octet-stream")));
        assert!(!is_html_content_type(None));
    }

    fn sample_archive_full(
        url: &str,
        order: u32,
        extract_to: &str,
        optional_group: Option<&str>,
        raw_filename: Option<&str>,
    ) -> ArchiveExtraction {
        ArchiveExtraction {
            url: url.to_string(),
            label: None,
            verification: ArtifactVerification::Sha256 { hash: "0".repeat(64) },
            order,
            extract_to: extract_to.to_string(),
            optional_group: optional_group.map(str::to_string),
            raw_filename: raw_filename.map(str::to_string),
            path_overrides: Vec::new(),
        }
    }

    #[test]
    fn dirty_shared_dest_dirs_includes_group_when_one_member_dirty() {
        let a = sample_archive_full("https://cdn.example.com/a.7z", 1, "SampleApp", None, None);
        let b = sample_archive_full("https://cdn.example.com/b.7z", 2, "SampleApp", None, None);
        let archives_with_dirty = [(&a, true), (&b, false)];
        let dirty = dirty_shared_dest_dirs(Path::new("/root"), &archives_with_dirty);
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty.get(&Path::new("/root").join("SampleApp")), Some(&1));
    }

    #[test]
    fn dirty_shared_dest_dirs_keeps_minimum_order_when_multiple_dirty() {
        let a = sample_archive_full("https://cdn.example.com/a.7z", 1, "SampleApp", None, None);
        let b = sample_archive_full("https://cdn.example.com/b.7z", 2, "SampleApp", None, None);
        let c = sample_archive_full("https://cdn.example.com/c.7z", 3, "SampleApp", None, None);
        // b(order 2)와 c(order 3)만 dirty해도, 그룹의 "최소 dirty order"는 2여야 한다
        // — a(order 1)는 그보다 낮으니 강제 대상이 아니게 하는 기준.
        let archives_with_dirty = [(&a, false), (&b, true), (&c, true)];
        let dirty = dirty_shared_dest_dirs(Path::new("/root"), &archives_with_dirty);
        assert_eq!(dirty.get(&Path::new("/root").join("SampleApp")), Some(&2));
    }

    #[test]
    fn dirty_shared_dest_dirs_empty_when_none_dirty() {
        let a = sample_archive_full("https://cdn.example.com/a.7z", 1, "SampleApp", None, None);
        let b = sample_archive_full("https://cdn.example.com/b.7z", 2, "SampleApp", None, None);
        let archives_with_dirty = [(&a, false), (&b, false)];
        assert!(dirty_shared_dest_dirs(Path::new("/root"), &archives_with_dirty).is_empty());
    }

    #[test]
    fn dirty_shared_dest_dirs_ignores_optional_group_and_raw_filename_archives() {
        // 그룹/raw_filename 압축은 둘 다 폴더 자체가 화이트리스트라(pruning 자체를
        // skip) dest_dir 를 통째로 신뢰하므로 이 그룹화 대상이 아니다.
        let grouped = sample_archive_full("https://cdn.example.com/c.7z", 1, "Content", Some("groupa"), None);
        let raw = sample_archive_full("https://cdn.example.com/tool.jar", 2, ".minecraft", None, Some("tool.jar"));
        let archives_with_dirty = [(&grouped, true), (&raw, true)];
        assert!(dirty_shared_dest_dirs(Path::new("/root"), &archives_with_dirty).is_empty());
    }

    #[test]
    fn archive_must_run_forces_higher_order_sibling_when_lower_order_dirty() {
        let b = sample_archive_full("https://cdn.example.com/b.7z", 2, "SampleApp", None, None);
        let dest_dir = Path::new("/root").join("SampleApp");
        let mut dirty_dest_dirs = HashMap::new();
        dirty_dest_dirs.insert(dest_dir.clone(), 1); // order 1(b 보다 낮음)이 dirty
        // b 자신의 마커는 유효(own_marker_dirty=false)해도, 자기보다 order가 낮은
        // 형제가 더러우면 강제로 재적용돼야 한다.
        assert!(archive_must_run(&b, &dest_dir, &dirty_dest_dirs, false));
    }

    #[test]
    fn archive_must_run_skips_lower_order_sibling_when_only_higher_order_dirty() {
        let a = sample_archive_full("https://cdn.example.com/a.7z", 1, "SampleApp", None, None);
        let dest_dir = Path::new("/root").join("SampleApp");
        let mut dirty_dest_dirs = HashMap::new();
        dirty_dest_dirs.insert(dest_dir.clone(), 10); // order 10(a 보다 높음)이 dirty
        assert!(!archive_must_run(&a, &dest_dir, &dirty_dest_dirs, false));
    }

    #[test]
    fn archive_must_run_respects_own_marker_when_dest_dir_clean() {
        let a = sample_archive_full("https://cdn.example.com/a.7z", 1, "SampleApp", None, None);
        let dest_dir = Path::new("/root").join("SampleApp");
        let dirty_dest_dirs = HashMap::new();
        assert!(!archive_must_run(&a, &dest_dir, &dirty_dest_dirs, false));
        assert!(archive_must_run(&a, &dest_dir, &dirty_dest_dirs, true));
    }

    #[test]
    fn archive_must_run_ignores_dirty_group_for_grouped_archive() {
        // optional_group 압축은 dirty_dest_dirs 에 걸려도 강제 재적용 대상이 아니다 —
        // 자기 마커 유효성만 본다.
        let grouped = sample_archive_full("https://cdn.example.com/c.7z", 1, "Content", Some("groupa"), None);
        let dest_dir = Path::new("/root").join("Content");
        let mut dirty_dest_dirs = HashMap::new();
        dirty_dest_dirs.insert(dest_dir.clone(), 5); // grouped(order 1)보다 높아도 무관 — 애초에 그룹화 대상 아님
        assert!(!archive_must_run(&grouped, &dest_dir, &dirty_dest_dirs, false));
    }

    fn sample_recipe_with_archives(archives: Vec<ArchiveExtraction>) -> Recipe {
        Recipe {
            id: "sample".to_string(),
            name: "Sample".to_string(),
            recipe_info: Default::default(),
            archives,
            files: vec![],
            optional_groups: vec![],
            folder_rules: vec![],
            launch: LaunchAction::SpawnProcess {
                entry_point: "x.exe".to_string(),
                entry_args: vec![],
            },
        }
    }

    // --- unique_fs_path ---

    #[test]
    fn unique_fs_path_returns_as_is_when_no_conflict() {
        let dir = temp_test_dir("unique-fs-path-no-conflict");
        assert_eq!(unique_fs_path(&dir, "file.txt"), dir.join("file.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_fs_path_appends_number_on_conflict() {
        let dir = temp_test_dir("unique-fs-path-conflict");
        std::fs::write(dir.join("file.txt"), b"x").unwrap();
        assert_eq!(unique_fs_path(&dir, "file.txt"), dir.join("file (2).txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_fs_path_increments_past_multiple_conflicts() {
        let dir = temp_test_dir("unique-fs-path-multi-conflict");
        std::fs::write(dir.join("file.txt"), b"x").unwrap();
        std::fs::write(dir.join("file (2).txt"), b"x").unwrap();
        std::fs::write(dir.join("file (3).txt"), b"x").unwrap();
        assert_eq!(unique_fs_path(&dir, "file.txt"), dir.join("file (4).txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unique_fs_path_preserves_extensionless_filename() {
        let dir = temp_test_dir("unique-fs-path-no-ext");
        std::fs::write(dir.join("README"), b"x").unwrap();
        assert_eq!(unique_fs_path(&dir, "README"), dir.join("README (2)"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- sniff_archive_kind ---

    #[test]
    fn sniff_archive_kind_detects_zip_magic() {
        let dir = temp_test_dir("sniff-zip");
        let path = dir.join("a.bin");
        std::fs::write(&path, [0x50, 0x4B, 0x03, 0x04, 0x00, 0x00]).unwrap();
        assert_eq!(sniff_archive_kind(&path).unwrap(), ArchiveKind::Zip);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sniff_archive_kind_detects_7z_magic() {
        let dir = temp_test_dir("sniff-7z");
        let path = dir.join("a.bin");
        std::fs::write(&path, [0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]).unwrap();
        assert_eq!(sniff_archive_kind(&path).unwrap(), ArchiveKind::SevenZ);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sniff_archive_kind_rejects_unknown_magic() {
        let dir = temp_test_dir("sniff-unknown");
        let path = dir.join("a.bin");
        std::fs::write(&path, b"not an archive").unwrap();
        assert!(sniff_archive_kind(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- crc32_of_file ---

    #[test]
    fn crc32_of_file_matches_crc32fast_direct() {
        let dir = temp_test_dir("crc32-of-file");
        let path = dir.join("a.bin");
        std::fs::write(&path, b"hello pengport").unwrap();
        assert_eq!(crc32_of_file(&path).unwrap(), crc32fast::hash(b"hello pengport"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- detect_archive_conflicts ---

    fn recipe_with_passthrough_ask_folder(path: &str, ask_on_conflict: bool) -> Recipe {
        let mut recipe = sample_recipe_with_archives(vec![]);
        recipe.folder_rules.push(FolderRule {
            path: path.to_string(),
            mode: FolderRuleMode::Passthrough { ask_on_conflict },
        });
        recipe
    }

    #[test]
    fn detect_archive_conflicts_none_when_no_ask_folders() {
        let recipe = sample_recipe_with_archives(vec![]);
        let scanned = vec![ScannedEntry {
            rel_path: "save.dat".to_string(),
            dest_path: PathBuf::from("/root/saves/save.dat"),
            crc32: 123,
        }];
        assert!(detect_archive_conflicts(&scanned, &recipe, Path::new("/root")).is_empty());
    }

    #[test]
    fn detect_archive_conflicts_none_when_ask_disabled() {
        let recipe = recipe_with_passthrough_ask_folder("saves", false);
        let dir = temp_test_dir("archive-conflict-ask-disabled");
        std::fs::create_dir_all(dir.join("saves")).unwrap();
        std::fs::write(dir.join("saves/save.dat"), b"existing content").unwrap();
        let scanned = vec![ScannedEntry {
            rel_path: "saves/save.dat".to_string(),
            dest_path: dir.join("saves/save.dat"),
            crc32: crc32fast::hash(b"archive content"), // 디스크와 다른 내용
        }];
        assert!(detect_archive_conflicts(&scanned, &recipe, &dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_archive_conflicts_none_when_content_identical() {
        let recipe = recipe_with_passthrough_ask_folder("saves", true);
        let dir = temp_test_dir("archive-conflict-identical");
        std::fs::create_dir_all(dir.join("saves")).unwrap();
        std::fs::write(dir.join("saves/save.dat"), b"same content").unwrap();
        let scanned = vec![ScannedEntry {
            rel_path: "saves/save.dat".to_string(),
            dest_path: dir.join("saves/save.dat"),
            crc32: crc32fast::hash(b"same content"),
        }];
        assert!(detect_archive_conflicts(&scanned, &recipe, &dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_archive_conflicts_flags_when_content_differs() {
        let recipe = recipe_with_passthrough_ask_folder("saves", true);
        let dir = temp_test_dir("archive-conflict-differs");
        std::fs::create_dir_all(dir.join("saves")).unwrap();
        std::fs::write(dir.join("saves/save.dat"), b"user's own content").unwrap();
        let scanned = vec![ScannedEntry {
            rel_path: "saves/save.dat".to_string(),
            dest_path: dir.join("saves/save.dat"),
            crc32: crc32fast::hash(b"archive content"),
        }];
        let conflicts = detect_archive_conflicts(&scanned, &recipe, &dir);
        assert_eq!(conflicts, vec!["saves/save.dat".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_archive_conflicts_none_when_dest_missing() {
        let recipe = recipe_with_passthrough_ask_folder("saves", true);
        let dir = temp_test_dir("archive-conflict-missing");
        std::fs::create_dir_all(dir.join("saves")).unwrap(); // save.dat 자체는 없음
        let scanned = vec![ScannedEntry {
            rel_path: "saves/save.dat".to_string(),
            dest_path: dir.join("saves/save.dat"),
            crc32: 42,
        }];
        assert!(detect_archive_conflicts(&scanned, &recipe, &dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_archive_conflicts_none_when_path_outside_ask_folder() {
        let recipe = recipe_with_passthrough_ask_folder("saves", true);
        let dir = temp_test_dir("archive-conflict-outside");
        std::fs::create_dir_all(dir.join("other")).unwrap();
        std::fs::write(dir.join("other/file.txt"), b"existing").unwrap();
        let scanned = vec![ScannedEntry {
            rel_path: "other/file.txt".to_string(),
            dest_path: dir.join("other/file.txt"),
            crc32: crc32fast::hash(b"different"),
        }];
        assert!(detect_archive_conflicts(&scanned, &recipe, &dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- apply_path_overrides_to_scan ---

    #[test]
    fn apply_path_overrides_to_scan_remaps_dest_path_but_keeps_rel_path() {
        // 압축 안에서는 "SampleApp/Launcher.exe"로 보이지만, path_overrides(from: "SampleApp",
        // to: "")로 실제로는 루트에 최종적으로 놓인다 — 충돌 판정은 그 최종 위치를
        // 봐야 한다. 다만 rel_path(적용 시점 해결책 조회 키)는 자연 경로 그대로여야
        // extract_zip_with_progress 등이 여전히 찾을 수 있다.
        let root = Path::new("/root");
        let dest_dir = root.join("SampleApp");
        let scanned = vec![ScannedEntry {
            rel_path: "SampleApp/Launcher.exe".to_string(),
            dest_path: dest_dir.join("SampleApp/Launcher.exe"),
            crc32: 1,
        }];
        let overrides = vec![PathOverride { from: "SampleApp/".to_string(), to: "".to_string() }];
        let remapped = apply_path_overrides_to_scan(scanned, root, &overrides);
        assert_eq!(remapped.len(), 1);
        assert_eq!(remapped[0].dest_path, root.join("Launcher.exe"));
        assert_eq!(remapped[0].rel_path, "SampleApp/Launcher.exe"); // 안 바뀜
    }

    #[test]
    fn apply_path_overrides_to_scan_no_op_when_empty() {
        let root = Path::new("/root");
        let scanned = vec![ScannedEntry {
            rel_path: "a.txt".to_string(),
            dest_path: root.join("a.txt"),
            crc32: 1,
        }];
        let remapped = apply_path_overrides_to_scan(scanned, root, &[]);
        assert_eq!(remapped[0].dest_path, root.join("a.txt"));
    }

    #[test]
    fn apply_path_overrides_to_scan_leaves_unmatched_entries_untouched() {
        let root = Path::new("/root");
        let dest_dir = root.join("SampleApp");
        let scanned = vec![ScannedEntry {
            rel_path: "other/file.txt".to_string(),
            dest_path: dest_dir.join("other/file.txt"),
            crc32: 1,
        }];
        let overrides = vec![PathOverride { from: "SampleApp".to_string(), to: "".to_string() }];
        let remapped = apply_path_overrides_to_scan(scanned, root, &overrides);
        assert_eq!(remapped[0].dest_path, dest_dir.join("other/file.txt")); // 안 바뀜
    }

    // --- pending resolutions round-trip ---

    #[test]
    fn pending_resolutions_round_trip() {
        let dir = temp_test_dir("pending-resolutions-roundtrip");
        let resolutions = vec![
            ArchiveEntryResolution::Overwrite { path: "a.txt".to_string() },
            ArchiveEntryResolution::Skip { path: "b.txt".to_string() },
            ArchiveEntryResolution::Rename { path: "c.txt".to_string() },
        ];
        write_pending_resolutions(&dir, "hash1", &resolutions).unwrap();
        let read_back = read_pending_resolutions(&dir, "hash1").unwrap();
        assert_eq!(read_back.len(), 3);
        assert!(matches!(read_back[0], ArchiveEntryResolution::Overwrite { .. }));
        assert!(matches!(read_back[1], ArchiveEntryResolution::Skip { .. }));
        assert!(matches!(read_back[2], ArchiveEntryResolution::Rename { .. }));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_pending_resolutions_none_when_missing() {
        let dir = temp_test_dir("pending-resolutions-missing");
        assert!(read_pending_resolutions(&dir, "nonexistent").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- prune_orphaned_pending_conflicts ---

    #[test]
    fn prune_orphaned_pending_conflicts_removes_unmatched_hashes() {
        let dir = temp_test_dir("prune-orphaned-pending");
        let a = sample_archive_full("https://cdn.example.com/a.7z", 1, "A", None, None);
        let current_hash = archive_content_hash(&a).unwrap();
        let recipe = sample_recipe_with_archives(vec![a]);

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{current_hash}.download")), b"keep me").unwrap();
        std::fs::write(dir.join("stalehash123.download"), b"orphan").unwrap();
        std::fs::write(dir.join("stalehash123.resolutions.json"), b"[]").unwrap();

        prune_orphaned_pending_conflicts(&recipe, &dir);

        assert!(dir.join(format!("{current_hash}.download")).exists());
        assert!(!dir.join("stalehash123.download").exists());
        assert!(!dir.join("stalehash123.resolutions.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- extract_zip_with_progress resolutions handling ---

    fn write_test_zip(path: &Path, entries: &[(&str, &[u8])]) {
        use std::io::Write;
        let file = std::fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn extract_zip_with_progress_applies_skip_and_rename_resolutions() {
        let dir = temp_test_dir("extract-zip-resolutions");
        let zip_path = dir.join("archive.zip");
        write_test_zip(
            &zip_path,
            &[
                ("normal.txt", b"normal content" as &[u8]),
                ("skip_me.txt", b"skip content"),
                ("rename_me.txt", b"rename content"),
            ],
        );
        let dest_dir = dir.join("dest");
        std::fs::create_dir_all(&dest_dir).unwrap();
        // rename_me.txt는 이미 다른 내용으로 존재 — "이름 바꿔 복사"가 그걸 안
        // 건드리고 압축 내용은 새 이름으로 따로 씀을 확인하기 위함.
        std::fs::write(dest_dir.join("rename_me.txt"), b"pre-existing user content").unwrap();

        let file = std::fs::File::open(&zip_path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut resolutions = HashMap::new();
        resolutions.insert(
            "skip_me.txt".to_string(),
            ArchiveEntryResolution::Skip { path: "skip_me.txt".to_string() },
        );
        resolutions.insert(
            "rename_me.txt".to_string(),
            ArchiveEntryResolution::Rename { path: "rename_me.txt".to_string() },
        );

        let written =
            extract_zip_with_progress(&mut archive, &dest_dir, false, None, &resolutions, |_, _| {}).unwrap();

        assert!(dest_dir.join("normal.txt").exists());
        assert!(!dest_dir.join("skip_me.txt").exists());
        assert_eq!(std::fs::read(dest_dir.join("rename_me.txt")).unwrap(), b"pre-existing user content");
        assert_eq!(std::fs::read(dest_dir.join("rename_me (2).txt")).unwrap(), b"rename content");
        assert!(written.iter().any(|w| w == "normal.txt"));
        assert!(written.iter().any(|w| w == "rename_me (2).txt"));
        assert!(!written.iter().any(|w| w == "skip_me.txt"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ancestor_archives_finds_base_archive_covering_grouped_dest_dir() {
        let base = sample_archive_full("https://cdn.example.com/sampleapp.7z", 1, "", None, None);
        let groupa = sample_archive_full("https://cdn.example.com/groupa.7z", 2, "SampleApp/GroupA", Some("groupa"), None);
        let recipe = sample_recipe_with_archives(vec![base.clone(), groupa]);
        let root = Path::new("/root");
        let wiped = root.join("SampleApp").join("GroupA");

        let ancestors = ancestor_archives(&recipe, root, &wiped);
        assert_eq!(ancestors.len(), 1);
        assert_eq!(ancestors[0].url, base.url);
    }

    #[test]
    fn ancestor_archives_does_not_include_sibling_groups() {
        // 형제 그룹(GroupB)은 GroupA 삭제에 영향받으면 안 된다 — 서로 조상 관계가 아님.
        let base = sample_archive_full("https://cdn.example.com/sampleapp.7z", 1, "", None, None);
        let groupa = sample_archive_full("https://cdn.example.com/groupa.7z", 2, "SampleApp/GroupA", Some("groupa"), None);
        let groupb = sample_archive_full("https://cdn.example.com/groupb.7z", 3, "SampleApp/GroupB", Some("groupb"), None);
        let recipe = sample_recipe_with_archives(vec![base, groupa, groupb]);
        let root = Path::new("/root");
        let wiped = root.join("SampleApp").join("GroupA");

        let ancestors = ancestor_archives(&recipe, root, &wiped);
        assert!(!ancestors.iter().any(|a| a.url.contains("groupb")));
    }

    #[test]
    fn ancestor_archives_excludes_self() {
        let groupa = sample_archive_full("https://cdn.example.com/groupa.7z", 1, "SampleApp/GroupA", Some("groupa"), None);
        let recipe = sample_recipe_with_archives(vec![groupa]);
        let root = Path::new("/root");
        let wiped = root.join("SampleApp").join("GroupA");

        assert!(ancestor_archives(&recipe, root, &wiped).is_empty());
    }

    #[test]
    fn ancestor_archives_empty_when_no_nesting() {
        let a = sample_archive_full("https://cdn.example.com/a.7z", 1, "A", None, None);
        let b = sample_archive_full("https://cdn.example.com/b.7z", 2, "B", None, None);
        let recipe = sample_recipe_with_archives(vec![a, b]);
        let root = Path::new("/root");
        let wiped = root.join("B");

        assert!(ancestor_archives(&recipe, root, &wiped).is_empty());
    }

    fn temp_test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pengport-library-test-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn hash_file_and_verify_accepts_matching_content() {
        let dir = temp_test_dir("hash-verify-match");
        let path = dir.join("file.bin");
        std::fs::write(&path, b"hello pengport").unwrap();
        let mut hasher = Sha256Verifier::new();
        hasher.update(b"hello pengport");
        let hash = hasher.finalize_hex();

        let result = hash_file_and_verify(&path, &ArtifactVerification::Sha256 { hash });

        assert!(result.is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hash_file_and_verify_rejects_mismatched_content() {
        let dir = temp_test_dir("hash-verify-mismatch");
        let path = dir.join("file.bin");
        std::fs::write(&path, b"actual downloaded bytes").unwrap();

        let result = hash_file_and_verify(&path, &ArtifactVerification::Sha256 { hash: "0".repeat(64) });

        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_round_trip() {
        let dir = temp_test_dir("manifest-roundtrip");
        let markers_dir = dir.join(".pengport-markers");
        write_manifest(&markers_dir, "abc123", &["a.txt".to_string(), "b/c.txt".to_string()]).unwrap();
        assert_eq!(
            read_manifest(&markers_dir, "abc123"),
            Some(vec!["a.txt".to_string(), "b/c.txt".to_string()])
        );
        remove_manifest_file(&markers_dir, "abc123");
        assert!(read_manifest(&markers_dir, "abc123").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_manifest_returns_none_when_missing() {
        let dir = temp_test_dir("manifest-missing");
        assert!(read_manifest(&dir, "nonexistent").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn sample_recipe_file(path: &str) -> RecipeFile {
        RecipeFile { path: path.to_string(), override_content: None, optional_group: None }
    }

    fn sample_recipe_with_files(files: Vec<RecipeFile>) -> Recipe {
        Recipe {
            id: "sample".to_string(),
            name: "Sample".to_string(),
            recipe_info: Default::default(),
            archives: vec![],
            files,
            optional_groups: vec![],
            folder_rules: vec![],
            launch: LaunchAction::SpawnProcess { entry_point: "x.exe".to_string(), entry_args: vec![] },
        }
    }

    fn sample_literal_text_file(path: &str, text: &str) -> RecipeFile {
        RecipeFile {
            path: path.to_string(),
            override_content: Some(OverrideContent::Literal {
                content: FileContent::Text { content: text.to_string() },
            }),
            optional_group: None,
        }
    }

    // --- 적용 지문 / declined 마커 ---

    #[test]
    fn path_fingerprint_round_trip() {
        let dir = temp_test_dir("fingerprint-roundtrip");
        let markers_dir = dir.join(".pengport-markers");
        write_path_fingerprint(&markers_dir, "SampleApp/option.ini", b"[GRAPHICS]\n3D_Mode=0\n").unwrap();
        assert_eq!(
            read_path_fingerprint(&markers_dir, "SampleApp/option.ini"),
            Some(sha256_hex(b"[GRAPHICS]\n3D_Mode=0\n")),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_path_fingerprint_none_when_missing() {
        let dir = temp_test_dir("fingerprint-missing");
        let markers_dir = dir.join(".pengport-markers");
        assert!(read_path_fingerprint(&markers_dir, "no/such/path.txt").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_resolved_true_when_done_marker_exists() {
        let dir = temp_test_dir("resolved-done");
        let markers_dir = dir.join(".pengport-markers");
        write_marker(&markers_dir, "abc").unwrap();
        assert!(is_resolved(&markers_dir, "abc"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_resolved_true_when_declined_marker_exists() {
        let dir = temp_test_dir("resolved-declined");
        let markers_dir = dir.join(".pengport-markers");
        write_declined_marker(&markers_dir, "abc").unwrap();
        assert!(is_resolved(&markers_dir, "abc"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_resolved_false_when_neither_marker_exists() {
        let dir = temp_test_dir("resolved-neither");
        let markers_dir = dir.join(".pengport-markers");
        assert!(!is_resolved(&markers_dir, "abc"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- detect_override_conflicts ---

    #[test]
    fn detect_override_conflicts_none_when_fingerprint_missing() {
        // 이 path를 이 메커니즘으로 관리한 적 없음(첫 설치 포함) — 비교 대상 자체가
        // 없어 충돌이 아니다.
        let dir = temp_test_dir("conflict-no-fingerprint");
        let root = dir.join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("option.ini"), b"whatever is already there").unwrap();
        let markers_dir = dir.join(".pengport-markers");
        let recipe =
            sample_recipe_with_files(vec![sample_literal_text_file("option.ini", "[GRAPHICS]\n3D_Mode=1\n")]);

        let conflicts = detect_override_conflicts(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();
        assert!(conflicts.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_override_conflicts_none_when_disk_matches_fingerprint() {
        let dir = temp_test_dir("conflict-matches");
        let root = dir.join("root");
        std::fs::create_dir_all(&root).unwrap();
        let bytes = b"[GRAPHICS]\n3D_Mode=1\n";
        std::fs::write(root.join("option.ini"), bytes).unwrap();
        let markers_dir = dir.join(".pengport-markers");
        write_path_fingerprint(&markers_dir, "option.ini", bytes).unwrap();
        let recipe =
            sample_recipe_with_files(vec![sample_literal_text_file("option.ini", "[GRAPHICS]\n3D_Mode=2\n")]);

        let conflicts = detect_override_conflicts(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();
        assert!(conflicts.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_override_conflicts_flags_when_disk_differs_from_fingerprint() {
        let dir = temp_test_dir("conflict-drift");
        let root = dir.join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("option.ini"), b"[GRAPHICS]\n3D_Mode=9\n").unwrap(); // 유저가 직접 바꿈
        let markers_dir = dir.join(".pengport-markers");
        write_path_fingerprint(&markers_dir, "option.ini", b"[GRAPHICS]\n3D_Mode=1\n").unwrap(); // 마지막으로 PengPort가 쓴 값
        let recipe =
            sample_recipe_with_files(vec![sample_literal_text_file("option.ini", "[GRAPHICS]\n3D_Mode=2\n")]); // 새 선언값

        let conflicts = detect_override_conflicts(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].path, "option.ini");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_override_conflicts_none_when_already_resolved() {
        let dir = temp_test_dir("conflict-resolved");
        let root = dir.join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("option.ini"), b"drifted content").unwrap();
        let markers_dir = dir.join(".pengport-markers");
        write_path_fingerprint(&markers_dir, "option.ini", b"original written content").unwrap();
        let recipe =
            sample_recipe_with_files(vec![sample_literal_text_file("option.ini", "[GRAPHICS]\n3D_Mode=2\n")]);
        let hash = file_override_hash(&recipe.files[0]).unwrap();
        write_declined_marker(&markers_dir, &hash).unwrap(); // 이미 "업데이트하지 않기"를 고름

        let conflicts = detect_override_conflicts(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();
        assert!(conflicts.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_override_conflicts_none_when_disk_file_missing() {
        let dir = temp_test_dir("conflict-file-missing");
        let root = dir.join("root"); // option.ini 자체를 안 만듦(지워졌다고 가정)
        std::fs::create_dir_all(&root).unwrap();
        let markers_dir = dir.join(".pengport-markers");
        write_path_fingerprint(&markers_dir, "option.ini", b"original written content").unwrap();
        let recipe =
            sample_recipe_with_files(vec![sample_literal_text_file("option.ini", "[GRAPHICS]\n3D_Mode=2\n")]);

        let conflicts = detect_override_conflicts(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();
        assert!(conflicts.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- adopt_disk_content ---

    #[test]
    fn adopt_disk_content_wraps_utf8_bytes_as_text() {
        let content = adopt_disk_content(b"[GRAPHICS]\n3D_Mode=9\n".to_vec()).unwrap();
        match content {
            FileContent::Text { content } => assert_eq!(content, "[GRAPHICS]\n3D_Mode=9\n"),
        }
    }

    #[test]
    fn adopt_disk_content_rejects_non_utf8_bytes() {
        let invalid_utf8 = vec![0xFF, 0xFE, 0x00];
        let err = adopt_disk_content(invalid_utf8).unwrap_err();
        assert!(err.contains("텍스트가 아니라"));
    }

    // --- OverrideConflictResolution wire format ---

    #[test]
    fn override_conflict_resolution_overwrite_ignores_extra_path_field() {
        // frontend는 균일한 배열을 위해 모든 액션에 path를 같이 보내지만, Overwrite는
        // Rust 쪽에 그 필드가 없다 — serde가 모르는 JSON 필드를 조용히 무시하는지 확인.
        let json = r#"{"action":"overwrite","path":"option.ini"}"#;
        let resolution: OverrideConflictResolution = serde_json::from_str(json).unwrap();
        assert!(matches!(resolution, OverrideConflictResolution::Overwrite));
    }

    #[test]
    fn prune_disallowed_files_deletes_pengport_markers_when_not_excluded() {
        let dest_dir = temp_test_dir("prune-markers-bug");
        std::fs::create_dir_all(dest_dir.join(".pengport-markers")).unwrap();
        std::fs::write(dest_dir.join(".pengport-markers/somehash.done"), "").unwrap();
        std::fs::write(dest_dir.join("Launcher.exe"), b"game").unwrap();

        let allowed: HashSet<String> = ["Launcher.exe".to_string()].into_iter().collect();
        prune_disallowed_files(&dest_dir, "", &allowed, &[]).unwrap();

        assert!(!dest_dir.join(".pengport-markers/somehash.done").exists());
        let _ = std::fs::remove_dir_all(&dest_dir);
    }

    #[test]
    fn prune_disallowed_files_preserves_pengport_markers_when_excluded() {
        let dest_dir = temp_test_dir("prune-markers-fixed");
        let markers_dir = dest_dir.join(".pengport-markers");
        std::fs::create_dir_all(&markers_dir).unwrap();
        std::fs::write(markers_dir.join("somehash.done"), "").unwrap();
        std::fs::write(dest_dir.join("Launcher.exe"), b"game").unwrap();

        let allowed: HashSet<String> = ["Launcher.exe".to_string()].into_iter().collect();
        prune_disallowed_files(&dest_dir, "", &allowed, std::slice::from_ref(&markers_dir)).unwrap();

        assert!(markers_dir.join("somehash.done").exists());
        assert!(dest_dir.join("Launcher.exe").exists());
        let _ = std::fs::remove_dir_all(&dest_dir);
    }

    fn recipe_with_folder_rule(mode: FolderRuleMode) -> Recipe {
        let mut recipe = sample_recipe_with_archives(vec![]);
        recipe.folder_rules.push(FolderRule { path: "SampleApp/saves".to_string(), mode });
        recipe
    }

    #[test]
    fn folder_rule_dest_dirs_includes_every_declared_path_regardless_of_mode() {
        let recipe = recipe_with_folder_rule(FolderRuleMode::Passthrough { ask_on_conflict: false });
        let root = Path::new("/root");
        assert_eq!(folder_rule_dest_dirs(&recipe, root), vec![root.join("SampleApp/saves")]);
    }

    #[test]
    fn apply_folder_rules_keeps_pattern_matched_and_declared_files() {
        let root = temp_test_dir("apply-folder-rules-keep");
        std::fs::create_dir_all(root.join("SampleApp/saves")).unwrap();
        std::fs::write(root.join("SampleApp/saves/slot1.sav"), b"save").unwrap();
        std::fs::write(root.join("SampleApp/saves/notes.txt"), b"declared").unwrap();

        let mut recipe = recipe_with_folder_rule(FolderRuleMode::Filtered {
            patterns: BTreeSet::from(["*.sav".to_string()]),
            disallow_patterns: BTreeSet::new(),
        });
        recipe.files.push(sample_recipe_file("SampleApp/saves/notes.txt"));
        let markers_dir = root.join(".pengport-markers");

        apply_folder_rules(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();

        assert!(root.join("SampleApp/saves/slot1.sav").exists());
        assert!(root.join("SampleApp/saves/notes.txt").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_folder_rules_disallow_removes_pattern_matched_file() {
        let root = temp_test_dir("apply-folder-rules-disallow-pattern");
        std::fs::create_dir_all(root.join("SampleApp/saves")).unwrap();
        std::fs::write(root.join("SampleApp/saves/slot1.sav"), b"save").unwrap();
        std::fs::write(root.join("SampleApp/saves/slot1.sav.bak"), b"backup").unwrap();

        let recipe = recipe_with_folder_rule(FolderRuleMode::Filtered {
            patterns: BTreeSet::from(["*.sav".to_string(), "*.sav.bak".to_string()]),
            disallow_patterns: BTreeSet::from(["*.bak".to_string()]),
        });
        let markers_dir = root.join(".pengport-markers");

        apply_folder_rules(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();

        // patterns 만으로는 둘 다 허용되지만, disallow_patterns 가 .bak 을 좁혀서 지운다.
        assert!(root.join("SampleApp/saves/slot1.sav").exists());
        assert!(!root.join("SampleApp/saves/slot1.sav.bak").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_folder_rules_disallow_never_removes_declared_file() {
        let root = temp_test_dir("apply-folder-rules-disallow-declared");
        std::fs::create_dir_all(root.join("SampleApp/saves")).unwrap();
        std::fs::write(root.join("SampleApp/saves/important.bak"), b"declared").unwrap();

        let mut recipe = recipe_with_folder_rule(FolderRuleMode::Filtered {
            patterns: BTreeSet::new(),
            disallow_patterns: BTreeSet::from(["*.bak".to_string()]),
        });
        recipe.files.push(sample_recipe_file("SampleApp/saves/important.bak"));
        let markers_dir = root.join(".pengport-markers");

        apply_folder_rules(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();

        // 명시적으로 선언된 파일은 disallow_patterns 가 넓게 걸려도 절대 안 지워진다.
        assert!(root.join("SampleApp/saves/important.bak").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_folder_rules_deletes_files_not_matching_pattern_or_declared() {
        let root = temp_test_dir("apply-folder-rules-reject");
        std::fs::create_dir_all(root.join("SampleApp/saves")).unwrap();
        std::fs::write(root.join("SampleApp/saves/malware.exe"), b"nope").unwrap();

        let recipe = recipe_with_folder_rule(FolderRuleMode::Filtered {
            patterns: BTreeSet::from(["*.sav".to_string()]),
            disallow_patterns: BTreeSet::new(),
        });
        let markers_dir = root.join(".pengport-markers");

        apply_folder_rules(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();

        assert!(!root.join("SampleApp/saves/malware.exe").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_folder_rules_passthrough_never_deletes() {
        let root = temp_test_dir("apply-folder-rules-passthrough");
        std::fs::create_dir_all(root.join("SampleApp/saves")).unwrap();
        std::fs::write(root.join("SampleApp/saves/anything.bin"), b"whatever").unwrap();

        let recipe = recipe_with_folder_rule(FolderRuleMode::Passthrough { ask_on_conflict: false });
        let markers_dir = root.join(".pengport-markers");

        apply_folder_rules(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();

        assert!(root.join("SampleApp/saves/anything.bin").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_folder_rules_skips_after_first_marker() {
        let root = temp_test_dir("apply-folder-rules-once");
        std::fs::create_dir_all(root.join("SampleApp/saves")).unwrap();
        std::fs::write(root.join("SampleApp/saves/malware.exe"), b"nope").unwrap();

        let recipe = recipe_with_folder_rule(FolderRuleMode::Filtered {
            patterns: BTreeSet::from(["*.sav".to_string()]),
            disallow_patterns: BTreeSet::new(),
        });
        let markers_dir = root.join(".pengport-markers");

        // 1회차 — 마커 없음, 실제로 정리됨.
        apply_folder_rules(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();
        assert!(!root.join("SampleApp/saves/malware.exe").exists());

        // 이후 사용자가 이 폴더에 뭘 넣어도(예: 앱이 런타임에 생성) 다음 호출은 안 건드림.
        std::fs::write(root.join("SampleApp/saves/new_stuff.exe"), b"created after first apply").unwrap();
        apply_folder_rules(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();
        assert!(root.join("SampleApp/saves/new_stuff.exe").exists());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn apply_folder_rules_skips_missing_folder() {
        let root = temp_test_dir("apply-folder-rules-missing");
        let recipe = recipe_with_folder_rule(FolderRuleMode::Filtered {
            patterns: BTreeSet::from(["*.sav".to_string()]),
            disallow_patterns: BTreeSet::new(),
        });
        let markers_dir = root.join(".pengport-markers");

        // 폴더가 아직 존재하지 않아도(아무 압축도 안 채움) 에러 없이 그냥 넘어가야 함.
        apply_folder_rules(&recipe, &HashSet::new(), &root, &markers_dir).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_declared_files_reports_only_absent_ones_under_extract_to() {
        let root = temp_test_dir("missing-declared-files");
        std::fs::create_dir_all(root.join("SampleApp")).unwrap();
        std::fs::write(root.join("SampleApp/Launcher.exe"), b"exists").unwrap();
        // "SampleApp/Other.exe"는 일부러 안 만듦 — 실제로 없는 케이스.

        let present = sample_recipe_file("SampleApp/Launcher.exe");
        let missing = sample_recipe_file("SampleApp/Other.exe");
        let outside_scope = sample_recipe_file("Other/unrelated.txt");
        let effective: Vec<&RecipeFile> = vec![&present, &missing, &outside_scope];

        let cache = build_dir_listing_cache(&root, &effective);
        let result = missing_declared_files("SampleApp", &effective, &cache);
        assert_eq!(result, vec!["SampleApp/Other.exe".to_string()]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_declared_files_empty_extract_to_matches_everything() {
        let root = temp_test_dir("missing-declared-files-root-scope");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("present.txt"), b"x").unwrap();

        let present = sample_recipe_file("present.txt");
        let missing = sample_recipe_file("missing.txt");
        let effective: Vec<&RecipeFile> = vec![&present, &missing];

        let cache = build_dir_listing_cache(&root, &effective);
        let result = missing_declared_files("", &effective, &cache);
        assert_eq!(result, vec!["missing.txt".to_string()]);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// 부모 디렉토리별 `read_dir` 로 묶어 처리하는 최적화가 여러 디렉토리에 걸친
    /// 선언들을 정확히 가려내는지 확인 — 존재하는 디렉토리(파일 일부 있음/전부 있음),
    /// 아예 없는 디렉토리(부모 자체가 없음)를 섞어서 검증.
    #[test]
    fn missing_declared_files_handles_multiple_parent_directories() {
        let root = temp_test_dir("missing-declared-files-multi-parent");
        std::fs::create_dir_all(root.join("SampleApp/GroupA")).unwrap();
        std::fs::write(root.join("SampleApp/GroupA/item1.dat"), b"x").unwrap();
        // SampleApp/GroupB 디렉토리는 아예 안 만듦 — 부모 자체가 없는 케이스.

        let files = [
            sample_recipe_file("SampleApp/GroupA/item1.dat"), // 존재
            sample_recipe_file("SampleApp/GroupA/item2.dat"), // 부모는 있지만 파일 없음
            sample_recipe_file("SampleApp/GroupB/item3.dat"), // 부모부터 없음
        ];
        let effective: Vec<&RecipeFile> = files.iter().collect();

        let cache = build_dir_listing_cache(&root, &effective);
        let mut result = missing_declared_files("SampleApp", &effective, &cache);
        result.sort();
        assert_eq!(result, vec!["SampleApp/GroupA/item2.dat".to_string(), "SampleApp/GroupB/item3.dat".to_string()]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remove_grouped_archive_content_with_manifest_preserves_untracked_files() {
        let root = temp_test_dir("manifest-precise");
        let dest_dir = root.join("SampleApp").join("GroupA");
        std::fs::create_dir_all(&dest_dir).unwrap();
        std::fs::write(dest_dir.join("track2.dat"), b"chart").unwrap(); // GroupA.7z 소유
        std::fs::write(dest_dir.join("shared.dat"), b"shared bgm").unwrap(); // SampleApp.7z 소유

        let groupa = sample_archive_full("https://cdn.example.com/groupa.7z", 2, "SampleApp/GroupA", Some("groupa"), None);
        let recipe = sample_recipe_with_archives(vec![groupa.clone()]);
        let markers_dir = root.join(".pengport-markers");
        let hash = archive_content_hash(&groupa).unwrap();
        write_manifest(&markers_dir, &hash, &["track2.dat".to_string()]).unwrap();
        write_marker(&markers_dir, &hash).unwrap();

        remove_grouped_archive_content(&recipe, &root, &markers_dir, &groupa).unwrap();

        assert!(!dest_dir.join("track2.dat").exists(), "매니페스트에 있던 파일은 삭제돼야 함");
        assert!(dest_dir.join("shared.dat").exists(), "매니페스트에 없던(다른 압축 소유) 파일은 보존돼야 함");
        assert!(!marker_exists(&markers_dir, &hash));
        assert!(read_manifest(&markers_dir, &hash).is_none());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remove_grouped_archive_content_without_manifest_falls_back_to_full_wipe_and_invalidates_ancestor() {
        let root = temp_test_dir("manifest-fallback");
        let dest_dir = root.join("SampleApp").join("GroupA");
        std::fs::create_dir_all(&dest_dir).unwrap();
        std::fs::write(dest_dir.join("anything.ojm"), b"x").unwrap();

        let base = sample_archive_full("https://cdn.example.com/sampleapp.7z", 1, "", None, None);
        let groupa = sample_archive_full("https://cdn.example.com/groupa.7z", 2, "SampleApp/GroupA", Some("groupa"), None);
        let recipe = sample_recipe_with_archives(vec![base.clone(), groupa.clone()]);
        let markers_dir = root.join(".pengport-markers");
        let base_hash = archive_content_hash(&base).unwrap();
        write_marker(&markers_dir, &base_hash).unwrap();
        write_marker(&markers_dir, &archive_content_hash(&groupa).unwrap()).unwrap();
        // groupa 마커만 있고 매니페스트는 없음(레거시/유실 시뮬레이션).

        remove_grouped_archive_content(&recipe, &root, &markers_dir, &groupa).unwrap();

        assert!(!dest_dir.exists(), "매니페스트 없으면 폴더 통째로 지워야 함");
        assert!(!marker_exists(&markers_dir, &base_hash), "조상(base) 마커도 무효화돼야 함");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn check_cancelled_ok_when_flag_false() {
        let flag = AtomicBool::new(false);
        assert!(check_cancelled(&flag).is_ok());
    }

    #[test]
    fn check_cancelled_returns_sentinel_when_flag_true() {
        let flag = AtomicBool::new(true);
        let err = check_cancelled(&flag).unwrap_err();
        assert_eq!(err, INSTALL_CANCELLED_SENTINEL);
    }

    #[test]
    fn library_cancel_install_sets_registered_flag() {
        let recipe_id = "test-cancel-sets-flag";
        let flag = register_install_cancel_flag(recipe_id);
        assert!(!flag.load(Ordering::Relaxed));

        let found = library_cancel_install(recipe_id.to_string());

        assert!(found, "등록된 설치는 취소 대상으로 찾아져야 함");
        assert!(flag.load(Ordering::Relaxed), "취소 호출 후 플래그가 켜져야 함");

        install_cancel_flags().lock().unwrap().remove(recipe_id);
    }

    #[test]
    fn library_cancel_install_returns_false_when_not_installing() {
        let found = library_cancel_install("test-cancel-nonexistent-recipe".to_string());
        assert!(!found);
    }

    #[test]
    fn install_cancel_guard_removes_entry_on_drop() {
        let recipe_id = "test-cancel-guard-drop";
        register_install_cancel_flag(recipe_id);
        assert!(install_cancel_flags().lock().unwrap().contains_key(recipe_id));
        {
            let _guard = InstallCancelGuard(recipe_id.to_string());
        }
        assert!(!install_cancel_flags().lock().unwrap().contains_key(recipe_id));
    }
}

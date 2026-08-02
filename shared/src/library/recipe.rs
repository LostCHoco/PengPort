//! [`Recipe`] — 라이브러리 항목 하나의 전체 정의 (v8 스키마).
//!
//! 핵심 모델: 설치는 **다운로드**와 **오버라이드** 두 종류의 사실만 있다.
//! - [`ArchiveExtraction`]: 압축을 받아서 어디에 풀지. 압축 안에 뭐가 있는지 레시피는
//!   미리 다 몰라도 됨 — 대신 실행 시점에 [`Recipe::files`]가 화이트리스트 역할을 해서,
//!   선언 안 된 파일은 압축 해제 직후 즉시 삭제된다(예외 없음 — 다운로드 링크가 바뀌어
//!   다른 미러를 쓰게 되거나 압축이 여러 개로 쪼개져도, "레시피가 아는 파일만 남는다"는
//!   불변식은 항상 지켜짐).
//! - [`RecipeFile`]: 레시피가 아는 파일 하나의 **유일한 위치**(`path`)와, 있다면 그 위에
//!   덮어씌울 내용(`override_content`).
//!
//! **루트(어느 폴더 기준으로 경로를 해석할지)는 [`Recipe::launch`] 하나가 결정한다** —
//! `archives`/`files` 항목마다 따로 "App이냐 third-party 앱이냐"를 고르지 않는다. 실제
//! 데이터에서 이 둘은 항상 1:1이었다(third-party 앱으로 실행하는 레시피가 굳이 그
//! 앱과 무관한 App 전용 폴더에도 뭔가 쓸 이유가 없음 — 실행 시점에 그 폴더는 아무도
//! 안 읽는다). 그래서 루트를 레시피 전체에 하나만 두면 "레시피가 어디에 뭘 두는지"의
//! 유일한 진실이 [`Recipe::launch`] 하나로 좁혀지고, 항목마다 반복 입력할 것도 없다.
//! third-party 앱 데이터 영역 안의 하위 경로(예: `.gamedata/.pack-src`)는 별도
//! `sub_path` 필드 없이 `extract_to`/`path`에 그대로 적는다(상대경로 표현이라 손실 없이
//! 합쳐짐).
//!
//! 설치 이후 앱을 **사용**하면서 생기는 변화(캐시, 사용자가 앱 안에서 바꾼 설정 등)는
//! 레시피의 책임 범위 밖이다 — "설치/업데이트가 필요한가"는 항상 "이 정확한
//! archive/override를 성공적으로 적용한 적이 있는가"(원장/마커)로만 판정하고, 지금
//! 실제 파일 내용을 레시피 선언값과 실시간으로 비교하지 않는다(그 비교는 런타임에
//! 생기는 정상적인 변화까지 "업데이트 필요"로 오판하게 만든다 — 실제로 겪은 버그).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// 라이브러리 항목 하나 — 카탈로그도 인스턴스도 아닌, flat 라이브러리의 단위.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Recipe {
    /// fs 경로 component 로 쓰이므로 `ids::validate_service_id` 통과 필수
    /// (호출자가 임포트 시점에 검증 — 이 타입 자체는 강제 안 함, serde 경계 최소화).
    /// `name`에서 최초 생성 시 1회 slugify — 편집 UI에는 노출 안 함.
    pub id: String,

    pub name: String,

    #[serde(default)]
    pub recipe_info: RecipeInfo,

    /// 압축 다운로드 — 순서대로 전부 받아서 검증+압축 해제된 뒤, [`Recipe::files`]로
    /// 화이트리스트 정리된다. 대상 루트는 [`Recipe::launch`]가 결정.
    #[serde(default)]
    pub archives: Vec<ArchiveExtraction>,

    /// 레시피가 아는 파일 전체(화이트리스트 겸 오버라이드 대상 목록) — 순서대로
    /// `override_content`가 있는 것만 적용된다. 대상 루트는 [`Recipe::launch`]가 결정.
    #[serde(default)]
    pub files: Vec<RecipeFile>,

    /// 선택적으로 설치할 수 있는 그룹 선언(예: 리듬게임의 난이도별 채보 팩) — 부분
    /// 설치를 지원한다. `files` 중 [`RecipeFile::optional_group`]이 여기 선언된 id를
    /// 참조하는 항목만 "선택 안 하면 없어도 되는" 파일이 된다. 선택 상태 자체는
    /// 레시피(공유 데이터)가 아니라 로컬 전용(`LibraryEntry::selected_optional_groups`)
    /// — 어떤 그룹이 있는지는 공유하지만 사용자가 뭘 골랐는지는 공유 안 함.
    #[serde(default)]
    pub optional_groups: Vec<OptionalGroup>,

    /// 화이트리스트 정리(`Recipe::files` 기준 pruning)의 기본 동작을 폴더 단위로
    /// 완화하는 예외 선언 — 선언 안 된 폴더는 기존과 동일하게 순수 화이트리스트.
    #[serde(default)]
    pub folder_rules: Vec<FolderRule>,

    /// 실행 방법이자 — `archives`/`files`의 대상 루트를 결정하는 유일한 근거.
    pub launch: LaunchAction,
}

/// [`Recipe::folder_rules`] 항목 하나 — 특정 폴더의 화이트리스트 pruning 정책을
/// 기본값(선언된 `RecipeFile` 경로만 허용)과 다르게 선언한다. `path`는
/// [`RecipeFile::path`]와 같은 표현(대상 루트 기준 상대경로, 슬래시 구분)이고,
/// `Recipe.folder_rules` 안에서 유일해야 한다(`validate_recipe` 강제).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FolderRule {
    pub path: String,
    pub mode: FolderRuleMode,
}

/// [`FolderRule::mode`] — 이 폴더를 pruning 이 어떻게 다뤄야 하는지. 둘 다 "설치까지만
/// 관여, 설치 후 앱 사용으로 생기는 변화는 건드리지 않는다"는 원칙([`recipe`] 모듈
/// 설명 참고)을 지킨다 — 매 재설치마다 계속 강제하는 모드는 없다: `Filtered`도 이
/// 규칙 선언 자체가 바뀔 때만(마커 기반 — `player-launcher/src-tauri/src/commands/library.rs`
/// 참고) 딱 1회 정리하고, 그 뒤로는 [`FolderRuleMode::Passthrough`]와 동일하게 손을
/// 뗀다.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FolderRuleMode {
    /// 이 폴더 밑은 pruning 대상에서 완전히 제외 — 뭐가 있든 손대지 않는다.
    Passthrough,
    /// 선언된 `RecipeFile` 경로 ∪ `patterns`(이 폴더 기준 상대 글롭)에 매치되는 파일만
    /// 허용 집합에 포함, 나머지는 삭제. `patterns`가 비어있으면 순수 화이트리스트와
    /// 동일 동작. 이 규칙(경로+패턴)이 바뀌지 않는 한 다시 적용되지 않는다.
    ///
    /// `disallow_patterns`는 `patterns`로 들어온 허용을 좁히는 예외다 — 선언된
    /// `RecipeFile` 경로(명시적 화이트리스트)는 절대 덮어쓰지 않는다. 폴더 단위의
    /// 넓은 제외 패턴이 실수로 명시 등록된 파일까지 지우는 함정을 피하기 위한 설계
    /// 결정(2026-08, 사용자 확인) — "패턴으로 들어온 것 중 이건 빼줘"만 표현한다.
    Filtered {
        #[serde(default)]
        patterns: BTreeSet<String>,
        #[serde(default)]
        disallow_patterns: BTreeSet<String>,
    },
}

/// [`Recipe::optional_groups`] 항목 하나 — 부분 설치 가능한 그룹의 선언(사람이 읽을
/// 이름 포함). 선택 여부 자체는 여기 없다(로컬 전용 상태, `LibraryEntry` 참고) —
/// `default_selected`는 어디까지나 확인 다이얼로그에 미리 체크될 값일 뿐, 사용자
/// 확인 없이 자동으로 설치되지 않는다.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OptionalGroup {
    /// [`RecipeFile::optional_group`]이 참조하는 id. 기술적 제약 없는(파일시스템 경로
    /// 등으로 쓰이지 않는) 순수 문자열 키 — 프론트엔드가 `label`에서 한 번 파생해
    /// 채우고 이후 `label`이 바뀌어도 고정 유지(이미 설치된 사용자의 선택 상태가
    /// `id` 기준으로 저장되므로).
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub default_selected: bool,
}

/// 카드에 보여줄 정적 표시 정보.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RecipeInfo {
    #[serde(default)]
    pub icon_url: Option<String>,
    /// 카드 배경 이미지 — 순수 표시용(검증 대상 아님, `icon_url`과 동일 취급).
    #[serde(default)]
    pub background_url: Option<String>,
}

/// 설치 아티팩트의 신뢰 검증 방식.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ArtifactVerification {
    /// 다운로드물의 SHA256 해시가 정확히 일치해야 함. lowercase hex, 64자.
    Sha256 { hash: String },
}

/// 압축을 받아서 어디에 풀지 — 다운로드는 "무엇을 어디서 받을지"만 정하고, 압축 안의
/// 개별 파일이 정확히 뭔지는 모른다(그건 실행 시점에 [`Recipe::files`] 화이트리스트가
/// 담당 — 선언 안 된 파일은 즉시 삭제된다, 예외 없음).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArchiveExtraction {
    /// 직접 다운로드 링크 또는 "사람이 눌러서 받아야 하는" 페이지(구글 드라이브
    /// 공유 페이지 등) 둘 다 될 수 있다 — 구분해서 선언할 필요 없이 자동 감지:
    /// PengPort가 먼저 직접 받아보고, 응답이 실제 파일이 아니라 페이지(HTML)로
    /// 판명되면 그때 기본 브라우저로 열어 사람이 받게 한 뒤 다운로드 폴더를
    /// 감시해서 `verification` 해시와 일치하는 파일을 찾는다(`commands::library`
    /// 참고, Tauri 전용이라 여기 shared 크레이트엔 없음).
    pub url: String,
    /// 편집 화면 카드 목록에 표시할 이름 — 없으면 `url`의 마지막 경로 조각에서
    /// 유도한다(프론트 `archiveDisplayName`). 단축 URL(`short.example/AbCdEf` 등)은
    /// 그 조각이 알아볼 수 없는 문자열이 되므로, 그럴 때 사람이 읽을 이름을 직접
    /// 지정하는 용도(2026-08, 사용자 확인) — 설치 로직에는 전혀 관여 안 하는
    /// 순수 표시용 필드.
    #[serde(default)]
    pub label: Option<String>,
    pub verification: ArtifactVerification,
    /// 다운로드+적용 순서 — `Recipe.archives` 안에서 유일해야 한다(`validate_recipe`
    /// 강제). 배열에 적힌 순서가 아니라 이 값이 실제 실행 순서를 결정한다: 두 압축이
    /// 같은 목적지에 겹치는 파일을 만들면, 이 값이 더 큰 쪽이 최종적으로 남는다.
    /// 배열 순서에 암묵적으로 기대지 않고 항상 명시적으로 적어야 한다(레시피를 다시
    /// 읽을 때 "어느 게 이기는지"가 이 필드 하나로 바로 보여야 함).
    pub order: u32,
    /// [`Recipe::launch`]가 정한 루트 기준 상대 경로. `""`(기본값)이면 루트에 직접
    /// 풀림. 압축 안의 개별 파일 경로가 아니라 "압축 전체가 풀릴 디렉토리" —
    /// [`RecipeFile::path`]와 같은 개념(레시피가 아는 유일한 경로 표현)을 그대로
    /// 재사용한 것뿐, 별도 필드 종류가 아니다.
    #[serde(default)]
    pub extract_to: String,
    /// [`RecipeFile::optional_group`]과 같은 개념 — 없으면 항상 다운로드, 있으면 그
    /// 그룹이 선택됐을 때만 다운로드한다. 압축 자체가 특정 선택 그룹 전용인 경우
    /// (예: 채보 팩 하나가 통째로 별도 압축)에 쓴다 — 화이트리스트로 사후에 걸러내는
    /// 것과 달리, 애초에 안 받아도 되는 걸 안 받는다(불필요한 다운로드 방지).
    #[serde(default)]
    pub optional_group: Option<String>,
    /// 있으면 이 다운로드를 압축으로 취급하지 않고, 검증된 바이트를 그대로
    /// `extract_to`(디렉토리) 밑에 이 이름의 파일 하나로 배치한다 — 아이콘/실행파일/jar
    /// 같은 단일 파일 자산용. 없으면 기존대로 URL 확장자(.zip/.7z) 기준 압축 해제.
    /// `extract_to`의 의미(목적지 디렉토리) 자체는 두 경우 다 동일 — raw 일 땐 그 밑에
    /// 파일 하나만 놓일 뿐이다.
    #[serde(default)]
    pub raw_filename: Option<String>,
    /// 압축 내부 구조가 평평해서(원래 서로 다른 위치에 있던 파일들을 편의상 한
    /// 압축에 모아놓은 경우 등) `extract_to` 하나로는 표현 못 하는 개별 파일 재배치가
    /// 필요할 때 쓴다 — "이 압축 안의 이 파일은 저 위치로 간다"는 포트포워딩처럼
    /// **항상 명시적**(자동 추측 없음)인 규칙. `extract_to`가 대다수 파일을 처리하는
    /// 기본 규칙이라면, 이건 그중 예외 몇 개를 콕 집어 다시 보내는 것.
    #[serde(default)]
    pub path_overrides: Vec<PathOverride>,
}

/// [`ArchiveExtraction::path_overrides`] 항목 하나. `from`은 압축 안 경로(압축을 그대로
/// 열었을 때 보이는 경로 — `strip_root`가 적용되는 경우 그것까지 벗겨낸 뒤의 경로,
/// `extract_to` 적용 전). `to`는 [`Recipe::launch`]가 정한 루트 기준 최종 경로
/// ([`RecipeFile::path`]와 같은 표현 — 슬래시 구분, 대상 루트가 유일한 기준).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PathOverride {
    pub from: String,
    pub to: String,
}

/// 레시피가 아는 파일 하나 — 위치(`path`)가 유일한 진실이고, 있다면 그 위에
/// 덮어씌울 내용까지 이 한 항목이 전부 갖고 있다(다운로드와 오버라이드를 별도 배열로
/// 두고 id로 서로 참조하게 만들지 않음 — 같은 파일에 대한 사실이 두 곳에 흩어지면
/// 서로 다른 경로를 적는 실수가 구조적으로 가능해지기 때문).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RecipeFile {
    /// [`Recipe::launch`]가 정한 루트 기준 상대 경로.
    pub path: String,
    /// 없으면 압축 해제 결과 그대로(화이트리스트 멤버로만 존재). 있으면 그 내용으로
    /// 덮어쓰거나(Literal) 특정 key만 patch(ConfigPatch).
    #[serde(default)]
    pub override_content: Option<OverrideContent>,
    /// 없으면 항상 필수. 있으면 [`Recipe::optional_groups`]의 그 id 그룹에 속함 —
    /// 사용자가 그 그룹을 선택했을 때만 화이트리스트에 포함된다(선택 안 하면 압축
    /// 에서 나와도 기존 화이트리스트 정리 로직이 그냥 지운다 — 별도 삭제 로직 불필요).
    #[serde(default)]
    pub optional_group: Option<String>,
}

/// [`RecipeFile::override_content`] — 파일에 실제로 어떤 내용을 반영할지.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OverrideContent {
    /// 파일 전체를 이 내용으로 그대로 씀.
    Literal { content: FileContent },
    /// 기존 파일(압축 해제 등으로 이미 존재해야 함)의 특정 key만 patch — 다른 내용
    /// 보존. 예: option.ini 같은 설정 파일의 특정 key만 바꾸는 케이스. `patch`는 포맷 무관 공통 표현 — 중첩
    /// 객체(예: ini/toml은 `{섹션: {키: 값}}`, json은 그 자체 구조)로 "이 값들을
    /// 덮어써라"를 나타낸다.
    ConfigPatch {
        format: ConfigFileFormat,
        patch: serde_json::Value,
    },
}

/// 실행 방법 — 정확히 1개. `archives`/`files`의 대상 루트도 이게 결정한다(모듈 설명
/// 참고) — `SpawnProcess`면 앱 전용 폴더, `ThirdPartyAppLaunch`면 그 앱의 데이터 영역.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchAction {
    /// `apps/<id>/` 대상 실행 파일 spawn — `archives`/`files`도 이 폴더 기준.
    SpawnProcess {
        /// 앱 루트 폴더 기준 실행 파일 상대 경로 (예: `"SampleApp/Launcher.exe"`).
        entry_point: String,
        #[serde(default)]
        entry_args: Vec<String>,
    },
    /// third-party app(예: Prism)을 통해 실행 — `archives`/`files`도 그 앱의 인스턴스
    /// 데이터 영역 기준. 실행 인자는 없음 — third-party app 실행 방식은 앱마다 완전히
    /// 달라(예: Prism은 `--launch <id>`) 어차피 이 필드로 표현 못 하고, 실제로 쓴 적도
    /// 없었다(죽은 필드였음, 제거).
    ThirdPartyAppLaunch { app_id: String },
}

/// [`OverrideContent::Literal`]이 쓰는 정적 콘텐츠. 텍스트(instance.cfg, mmc-pack.json
/// 등)와 바이너리(servers.dat NBT 등) 둘 다 지원.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "encoding", rename_all = "snake_case")]
pub enum FileContent {
    Text { content: String },
    /// base64 인코딩된 바이너리 콘텐츠.
    Base64 { data: String },
}

/// [`OverrideContent::ConfigPatch`]의 대상 파일 포맷 — 순수 태그. 파싱/patch 적용 로직은
/// `player-launcher/src-tauri/src/commands/config_patch.rs`가 이 태그로 분기해서 처리
/// (레시피 데이터 쪽엔 포맷별 타입이 없음 — 새 포맷 추가는 그 분기 함수에 브랜치
/// 하나 추가하는 것으로 끝, 이 enum에 variant 하나 느는 것 외엔 스키마 변화 없음).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFileFormat {
    /// `[section]` + `key=value`. `patch`는 `{"섹션": {"키": 값}}`.
    Ini,
    /// `patch`를 그대로 파일의 최상위 구조에 재귀 병합.
    Json,
    /// TOML 테이블. `patch`는 ini와 동일하게 `{"테이블": {"키": 값}}`.
    Toml,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_third_party_recipe_json() -> &'static str {
        r#"
        {
          "id": "sample-service",
          "name": "샘플 서비스",
          "recipe_info": { "icon_url": "https://cdn.example.com/icon.png" },
          "files": [
            {
              "path": "instance.cfg",
              "override_content": { "kind": "literal", "content": { "encoding": "text", "content": "name=sample-service\n" } }
            },
            {
              "path": ".gamedata/data.bin",
              "override_content": { "kind": "literal", "content": { "encoding": "base64", "data": "AAAA" } }
            }
          ],
          "launch": {
            "kind": "third_party_app_launch",
            "app_id": "test_app"
          }
        }
        "#
    }

    #[test]
    fn deserialize_third_party_recipe() {
        let r: Recipe = serde_json::from_str(sample_third_party_recipe_json()).unwrap();
        assert_eq!(r.id, "sample-service");
        assert_eq!(
            r.recipe_info.icon_url.as_deref(),
            Some("https://cdn.example.com/icon.png")
        );
        assert_eq!(r.files.len(), 2);
        assert_eq!(r.files[0].path, "instance.cfg");
        match &r.launch {
            LaunchAction::ThirdPartyAppLaunch { app_id } => assert_eq!(app_id, "test_app"),
            other => panic!("expected ThirdPartyAppLaunch, got {other:?}"),
        }
    }

    /// 포터블 앱(직접 spawn) 사례 — 본체 압축 해제 + entry_point + 설정 파일 설치시 patch.
    #[test]
    fn deserialize_portable_app_recipe() {
        let json = r#"
        {
          "id": "sample-app",
          "name": "SampleApp",
          "archives": [
            {
              "url": "https://cdn.example.com/SampleApp.7z",
              "verification": { "kind": "sha256", "hash": "aaaa" },
              "order": 1,
              "extract_to": ""
            }
          ],
          "files": [
            { "path": "SampleApp/Launcher.exe" },
            {
              "path": "SampleApp/option.ini",
              "override_content": {
                "kind": "config_patch",
                "format": "ini",
                "patch": { "GRAPHICS": { "3D_Mode": "0" } }
              }
            }
          ],
          "launch": {
            "kind": "spawn_process",
            "entry_point": "SampleApp/Launcher.exe"
          }
        }
        "#;
        let r: Recipe = serde_json::from_str(json).unwrap();
        assert_eq!(r.archives.len(), 1);
        assert_eq!(r.archives[0].url, "https://cdn.example.com/SampleApp.7z");

        assert_eq!(r.files.len(), 2);
        assert!(r.files[0].override_content.is_none()); // 화이트리스트 멤버로만 존재
        match &r.files[1].override_content {
            Some(OverrideContent::ConfigPatch { format, patch }) => {
                assert_eq!(*format, ConfigFileFormat::Ini);
                assert_eq!(patch["GRAPHICS"]["3D_Mode"], "0");
            }
            other => panic!("expected ConfigPatch, got {other:?}"),
        }
        match &r.launch {
            LaunchAction::SpawnProcess { entry_point, entry_args } => {
                assert_eq!(entry_point, "SampleApp/Launcher.exe");
                assert!(entry_args.is_empty());
            }
            other => panic!("expected SpawnProcess, got {other:?}"),
        }
    }

    /// 추가 콘텐츠 팩(선택적 콘텐츠 케이스) — 압축이 여러 개라도 같은 `files`
    /// 화이트리스트를 공유한다(어느 압축에서 나왔는지는 구조적으로 안 이어져 있음).
    #[test]
    fn deserialize_multiple_archives_share_one_file_whitelist() {
        let json = r#"
        {
          "id": "sample-app",
          "name": "SampleApp",
          "archives": [
            { "url": "https://cdn.example.com/SampleApp.7z",
              "verification": { "kind": "sha256", "hash": "aaaa" }, "order": 1 },
            { "url": "https://cdn.example.com/Content.7z",
              "verification": { "kind": "sha256", "hash": "bbbb" }, "order": 2, "extract_to": "SampleApp" }
          ],
          "files": [
            { "path": "SampleApp/Launcher.exe" },
            { "path": "SampleApp/Content/data.bin" }
          ],
          "launch": { "kind": "spawn_process", "entry_point": "SampleApp/Launcher.exe" }
        }
        "#;
        let r: Recipe = serde_json::from_str(json).unwrap();
        assert_eq!(r.archives.len(), 2);
        // extract_to 생략 시 기본값.
        assert_eq!(r.archives[0].extract_to, "");
        assert_eq!(r.archives[1].extract_to, "SampleApp");
        assert_eq!(r.files.len(), 2);
    }

    /// modpack 번들처럼 third-party app 데이터 영역(하위 경로 포함)으로 다운로드하는
    /// 케이스 — `sub_path` 별도 필드 없이 `extract_to`에 그대로 적는다.
    #[test]
    fn deserialize_archive_with_nested_extract_to() {
        let json = r#"
        {
          "url": "https://cdn.example/pack.tar.gz",
          "verification": { "kind": "sha256", "hash": "cccc" },
          "order": 1,
          "extract_to": ".gamedata/.pack-src"
        }
        "#;
        let a: ArchiveExtraction = serde_json::from_str(json).unwrap();
        assert_eq!(a.extract_to, ".gamedata/.pack-src");
    }

    /// 옛 v7 데이터(archives/files 항목마다 target 필드가 있던 시절)를 열어도 깨지지
    /// 않아야 함 — 알 수 없는 필드는 무시(`#[serde(default)]` 없이도 serde 기본 동작).
    #[test]
    fn deserialize_ignores_legacy_target_field() {
        let json = r#"
        {
          "id": "sample-service",
          "name": "샘플 서비스",
          "archives": [
            { "url": "https://cdn.example/pack.zip", "verification": { "kind": "sha256", "hash": "aaaa" }, "order": 1,
              "target": { "kind": "third_party_app", "app_id": "test_app", "sub_path": ".gamedata/.pack-src" } }
          ],
          "files": [
            { "target": { "kind": "third_party_app", "app_id": "test_app", "sub_path": "" }, "path": "instance.cfg" }
          ],
          "launch": { "kind": "third_party_app_launch", "app_id": "test_app" }
        }
        "#;
        let r: Recipe = serde_json::from_str(json).unwrap();
        assert_eq!(r.id, "sample-service");
        assert_eq!(r.archives.len(), 1);
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files[0].path, "instance.cfg");
    }

    #[test]
    fn round_trip_serialize_deserialize() {
        let original: Recipe = serde_json::from_str(sample_third_party_recipe_json()).unwrap();
        let json = serde_json::to_string(&original).unwrap();
        let back: Recipe = serde_json::from_str(&json).unwrap();
        assert_eq!(original.id, back.id);
        assert_eq!(back.files.len(), 2);
    }

    #[test]
    fn archives_and_files_may_both_be_empty_at_type_level_but_runtime_rejects() {
        // 타입 레벨에서는 강제 안 함 — "최소 하나는 있어야" 는
        // `crate::actions::validate_recipe`가 담당. 여기선 역직렬화만 확인.
        let json = r#"
        {
          "id": "bad",
          "name": "Bad",
          "launch": { "kind": "spawn_process", "entry_point": "x.exe" }
        }
        "#;
        let r: Recipe = serde_json::from_str(json).unwrap();
        assert!(r.archives.is_empty());
        assert!(r.files.is_empty());
    }
}

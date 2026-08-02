//! Third-party app(예: Prism Launcher) — 데이터로 표현, 플러그인 시스템 불필요.
//!
//! 옛 버전은 "Prism 지원"을 `LaunchSpec::PrismInstance` variant + 전용 렌더링 코드로
//! 하드코딩했다. 실제로 뜯어보면 그 코드가 아는 건 "어디에 데이터가 있는지·어떻게
//! 받는지·어떻게 실행하는지"뿐이고, 그건 전부 [`ThirdPartyAppDescriptor`]라는 데이터로
//! 환원된다 — 위치(override/bundled/시스템 탐지), 자동 다운로드([`DownloadStrategy`]),
//! 실행 인자([`ThirdPartyAppDescriptor::launch_args_template`]), 준비 완료 판별
//! ([`ReadinessSignal`]) 넷 다. 설계 근거: `docs/design/RECIPE_MODEL.md` §4,
//! `docs/design/THIRD_PARTY_PLATFORM_MODEL.md`.
//!
//! **"새 third-party app 추가 = PengPort 코드 0줄"은 프리즘 하나로만 검증된 주장이다.**
//! 이 모델(포터블 zip 자동 다운로드 + `data_root/instances/<id>`에 파일을 직접 던져넣는
//! 방식)은 프리즘·MultiMC 계열(launcher-of-launcher, 파일드롭으로 인스턴스가 생김)에
//! 실제로 맞아떨어졌을 뿐이다. Steam처럼 (a) 포터블 zip이 아니라 실행형 설치 프로그램을
//! 쓰고 (b) 자기 라이브러리를 자체 DB로 관리해 PengPort가 폴더에 파일을 드롭하는
//! 방식으로 "설치"할 수 없는 앱은 지금 이 스키마로 표현이 안 된다 — 그런 앱을 실제로
//! 추가하려면 `DownloadStrategy`에 새 variant를 추가하는 등 진짜 코드 변경이 필요할
//! 가능성이 높다. 다음에 진짜 두 번째 third-party app을 추가할 때, 그 앱이 이 모델에
//! 맞는지부터 다시 확인할 것 — 안 맞으면 이 문서 전체를 그 앱 기준으로 재검토한다.
//!
//! (한때 exe 옆 포터블 번들 탐지 + 개발용 env override + 포터블 마커 파일 판별까지
//! 있었으나, 실사용 판단으로 제거됨 — 전자 둘은 "사용자 override 로 충분히 대체되는
//! 별도 경로"였고, 마커 파일 판별은 override/bundled 둘 다 "그 폴더 자체가 데이터
//! 루트"로 단순화하며 불필요해짐(단, override 로 시스템 설치본의 실행파일 위치를
//! 지정하면 데이터가 다른 곳에 있어 못 찾을 수 있음 — 의도적으로 감수한 단순화).
//! 시스템 탐지만은 "이미 설치된 걸 찾아주는 편의"가 실사용 가치가 있다고 판단해 유지.
//! `THIRD_PARTY_PLATFORM_MODEL.md` §5 참고.
//!
//! 시스템 탐지 자체도 한때는 앱마다 레지스트리 키 이름·설치 폴더 이름을 등록 폼에서
//! 직접 입력받는 `DetectionStrategy` 목록이었으나, 그 값들을 정확히 아는 사람이
//! 거의 없어(설치 프로그램을 직접 뜯어봐야 알 수 있는 정보) 폐기됨 — 지금은 [`detect`]
//! 가 `exe_filename` 하나만으로 항상 똑같이 동작하는 고정 알고리즘(레지스트리 전체
//! 열거 → 표준 폴더 전체 열거 → PATH)이라 앱별 설정이 필요 없다.)
//!
//! descriptor **값**의 저장 위치(컴파일타임 내장 vs 사용자 편집 가능한 로컬 데이터
//! 파일)는 이 크레이트의 책임이 아니다 — `crate::library::third_party_store`가
//! 레시피([`crate::library::LibraryStore`])와 같은 패턴으로 영속화한다.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// third-party app 을 실행한 뒤 "진짜로 준비됐다(=페이로드가 뜨고 사용자에게 보임)"를
/// 판별하는 방법 — 선언 안 하면 spawn 즉시 "실행 중"으로 취급(감시 자체를 생략).
///
/// 필요해지면 항목 추가 — [`DetectionStrategy`]와 같은 확장 방식(기존 필드를 늘리는
/// 게 아니라 이 리스트에 variant 를 추가). 지금은 `ChildProcessWindow` 하나뿐인데,
/// 이건 "Prism이 우연히 이렇게 동작해서"가 아니라 **Prism이 이 신호를 공식적으로
/// 전혀 제공하지 않기 때문에**(Prism 공식 문서 확인 — pre-launch/post-exit 커맨드는
/// 있지만 "게임이 지금 떴다" 훅은 없음) PengPort 가 프로세스 트리를 관찰해서 추론할
/// 수밖에 없는, 여러 launcher-of-launcher 형 앱에 재발할 법한 상황을 담은 variant다.
/// 다음 앱이 자체 신호(공식 API/IPC)를 준다면 그건 새 variant(예: `IpcSignal`)로
/// 추가하면 되고, 이 variant 자체를 억지로 넓히지 않는다.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadinessSignal {
    /// 실행한 프로세스(부모)의 자식 중 cmdline 에 `cmdline_contains` 가 포함된 게
    /// 나타나고, 그 자식이 visible top-level window 를 가지면 "준비됨"으로 판단.
    /// 부모가 실제 페이로드를 별도 자식 프로세스로 띄우면서 그 사실을 외부에 알려주지
    /// 않는 launcher-of-launcher 형 앱을 위함(Java 기반이면 `cmdline_contains` 에
    /// 메인 클래스 이름을 넣는 게 자연스럽지만, 임의의 자식 exe 이름/인자로도 매치 가능).
    ChildProcessWindow { cmdline_contains: String },
}

/// third-party app 자동 다운로드 방법 — 필요해지면 항목 추가(기존 필드를 늘리는 게
/// 아니라 이 리스트에 variant 를 추가). 다운로드된 아카이브는 항상 zip 으로 취급해
/// `%LOCALAPPDATA%\PengPort\<app_id>\`(bundled root)에 통째로 푼다 — 레시피의
/// `archives`(개별 파일 화이트리스트)와 달리 이건 서드파티 앱 그 자체를 통째로
/// 신뢰하는 영역이라 화이트리스트 정리를 하지 않는다(기존 `download_prism` 동작 유지).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DownloadStrategy {
    /// 고정 URL에서 받아 `verification`으로 검증. 레시피 아카이브와 같은 신뢰 모델.
    StaticUrl {
        url: String,
        verification: crate::library::ArtifactVerification,
    },
    /// GitHub 저장소의 최신 release 에서 `asset_name_pattern`(부분 문자열, 대소문자
    /// 구분)을 포함하는 `.zip` 자산 하나를 찾아 받는다. "최신"이라 release 마다 실제
    /// 바이트가 달라지므로 고정 해시 검증은 불가능 — HTTPS + GitHub 도메인 자체를
    /// 신뢰 근거로 삼는다(기존 `download_prism`이 이미 이렇게 동작했음, 회귀 아님).
    GithubLatestRelease {
        /// `"owner/repo"` 형식.
        repo: String,
        asset_name_pattern: String,
    },
}

/// third-party app 하나의 전체 정의 — 코드가 아니라 데이터. 잘못된 `id` 를 참조해도
/// "그 이름의 앱을 못 찾음"일 뿐 임의 코드 실행 위험이 없다.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThirdPartyAppDescriptor {
    /// `RecipeFile::target`(`DownloadTarget::ThirdPartyApp`)/`LaunchAction::ThirdPartyAppLaunch`
    /// 가 참조하는 id.
    pub id: String,
    /// 사람이 읽는 표시 이름(예: `"PrismLauncher"`) — 설정 화면(`ThirdPartyApps.tsx`)
    /// 카드 제목용. 없으면 `id`를 그대로 표시(개발 편의 — 이 필드를 안 채워도 최소한
    /// 뭔가는 보여야 하므로).
    #[serde(default)]
    pub label: Option<String>,
    pub exe_filename: String,
    /// 자동 다운로드 방법(선택) — 없으면 설정 화면에 자동 다운로드 버튼 자체가 안 뜬다.
    #[serde(default)]
    pub download_strategy: Option<DownloadStrategy>,
    /// 자동 다운로드 완료 후 전용 사본(bundled root) 바로 밑에 빈 내용으로 만들 파일
    /// 이름들(선택) — 일부 앱은 "이 폴더 옆에 이 마커 파일이 있으면 포터블 모드로
    /// 동작"하는 자체 규약을 갖는다(예: Prism 의 `portable.txt` — 없으면 시스템
    /// Prism 데이터(`%APPDATA%\PrismLauncher\`)와 데이터가 섞임). 그 규약 자체는
    /// 앱마다 다르지만 "빈 마커 파일 만들기"는 공통 동작이라 데이터로 표현.
    #[serde(default)]
    pub post_download_marker_files: Vec<String>,
    /// 데이터 루트 밑, 인스턴스들이 들어가는 하위 폴더 이름(예: `"instances"`).
    /// 인스턴스 데이터를 쓸 필요 없는(탐지만 필요한) 앱은 안 채워도 됨.
    #[serde(default)]
    pub instances_subfolder: Option<String>,
    /// 시스템 탐지로 찾았을 때의 데이터 루트 — `%APPDATA%\<이 이름>\`. 시스템 설치본은
    /// 항상 이 위치가 고정이라(포터블처럼 마커 파일로 판별할 필요 없음) 단순 문자열.
    #[serde(default)]
    pub system_appdata_folder_name: Option<String>,
    /// 실행 후 "진짜로 준비됐다"를 판별하는 방법(선택) — 없으면 spawn 즉시 실행 중으로
    /// 취급.
    #[serde(default)]
    pub readiness_signal: Option<ReadinessSignal>,
    /// 실행 인자 템플릿 — `{instance_id}` 자리표시자를 [`build_launch_args`]가 실제
    /// instance id 로 치환한다(예: `["--launch", "{instance_id}"]`). 비어있으면 인자
    /// 없이 exe 만 실행(자체적으로 인스턴스 개념이 없는 앱).
    #[serde(default)]
    pub launch_args_template: Vec<String>,
}

/// [`ThirdPartyAppDescriptor::launch_args_template`]의 `{instance_id}` 자리표시자를
/// 실제 instance id 로 치환. 앱마다 실행 인자 형태가 완전히 다르므로(Prism 은
/// `--launch <id>`) 이 함수는 문자열 치환만 하고 의미는 모른다.
pub fn build_launch_args(template: &[String], instance_id: &str) -> Vec<String> {
    template
        .iter()
        .map(|arg| arg.replace("{instance_id}", instance_id))
        .collect()
}

/// [`resolve_third_party_app`] 호출자가 미리 알고 있는, descriptor 만으로는 알 수 없는
/// 로컬 상태(설정 파일 등) — 이 크레이트는 Tauri/설정 파일 접근을 모르므로 호출자가
/// 채워서 넘긴다.
#[derive(Debug, Clone, Default)]
pub struct DataRootLookupContext {
    /// 사용자가 설정 화면에서 지정한 override 경로 — 그 폴더 자체가 데이터 루트로
    /// 취급된다(포터블 번들이 아니라 시스템 설치본의 실행파일 위치를 지정하면 데이터가
    /// 다른 곳에 있어 못 찾을 수 있음 — 의도적으로 감수한 단순화).
    pub user_override_root: Option<PathBuf>,
    /// PengPort 가 자동 설치(다운로드)해둔 위치 — PengPort 자신이 만든 위치라 항상
    /// 그 폴더 자체가 데이터 루트임이 보장된다.
    pub bundled_root: Option<PathBuf>,
}

/// 어느 경로 소스에서 찾았는지 — UI 표시/디버깅용.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThirdPartyAppSource {
    UserOverride,
    Bundled,
    /// 이미 설치된 걸 레지스트리/표준폴더/PATH 로 찾음.
    System,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedThirdPartyApp {
    pub exe: PathBuf,
    pub data_root: PathBuf,
    pub source: ThirdPartyAppSource,
}

/// descriptor + 로컬 컨텍스트로 실행파일과 데이터 루트를 함께 찾는다. 우선순위(고정
/// 알고리즘 — 앱이 달라져도 이 순서 자체는 범용): 사용자 override → PengPort 자동
/// 설치 위치 → 시스템 탐지. 앞의 둘은 "후보 루트 자체가 곧 데이터 루트"라 판별 로직이
/// 필요 없고, 시스템 탐지는 데이터가 항상 OS 표준 AppData 에 고정. 새 앱 추가는 이
/// 함수를 건드리지 않고 descriptor 데이터만 채우면 된다.
pub fn resolve_third_party_app(
    descriptor: &ThirdPartyAppDescriptor,
    ctx: &DataRootLookupContext,
) -> Option<ResolvedThirdPartyApp> {
    if let Some(root) = &ctx.user_override_root {
        let exe = root.join(&descriptor.exe_filename);
        if exe.is_file() {
            return Some(ResolvedThirdPartyApp {
                exe,
                data_root: root.clone(),
                source: ThirdPartyAppSource::UserOverride,
            });
        }
    }

    if let Some(root) = &ctx.bundled_root {
        let exe = root.join(&descriptor.exe_filename);
        if exe.is_file() {
            return Some(ResolvedThirdPartyApp {
                exe,
                data_root: root.clone(),
                source: ThirdPartyAppSource::Bundled,
            });
        }
    }

    if let (Some(exe), Some(folder_name)) =
        (detect(&descriptor.exe_filename), &descriptor.system_appdata_folder_name)
    {
        if let Some(data_root) = appdata_dir(folder_name) {
            return Some(ResolvedThirdPartyApp {
                exe,
                data_root,
                source: ThirdPartyAppSource::System,
            });
        }
    }

    None
}

fn appdata_dir(folder_name: &str) -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join(folder_name))
}

/// 시스템에 이미 설치된 `exe_filename`을 찾는다 — 앱마다 미리 채워둘 값이 없는 고정
/// 알고리즘(순서대로 시도, 첫 성공 반환): ①Uninstall 레지스트리 전체 열거(각 항목의
/// `InstallLocation`에 exe가 있는지) ②표준 설치 루트 바로 아래 모든 폴더 열거(각각에
/// exe가 있는지) ③`PATH`. 셋 다 `exe_filename` 하나로 충분 — 레지스트리 키 이름/설치
/// 폴더 이름처럼 앱마다 다른 값을 등록 시점에 몰라도 된다(과거엔 이런 값을 앱마다
/// 입력받는 `DetectionStrategy` 목록이었으나, 정확히 아는 사람이 거의 없어 폐기됨).
pub fn detect(exe_filename: &str) -> Option<PathBuf> {
    detect_via_registry(exe_filename)
        .or_else(|| detect_via_standard_folders(exe_filename))
        .or_else(|| detect_via_path(exe_filename))
}

/// `PATH` 환경변수의 각 디렉토리에서 `exe_filename` 을 찾는다.
fn detect_via_path(exe_filename: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(exe_filename);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Uninstall 레지스트리(`HKLM`/`HKCU`, 32bit 앱은 `WOW6432Node`)의 서브키를 전부 열거해
/// 각 `InstallLocation`에 `exe_filename`이 있는지 확인 — 특정 서브키 이름을 미리 알
/// 필요가 없다(설치 프로그램이 그 위치를 어디로 정했든 잡아낸다).
#[cfg(windows)]
fn detect_via_registry(exe_filename: &str) -> Option<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    let candidates = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (
            HKEY_LOCAL_MACHINE,
            r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
    ];

    for (hive, path) in candidates {
        let root = RegKey::predef(hive);
        let Ok(uninstall) = root.open_subkey_with_flags(path, KEY_READ) else {
            continue;
        };
        for name in uninstall.enum_keys().flatten() {
            let Ok(entry) = uninstall.open_subkey_with_flags(&name, KEY_READ) else {
                continue;
            };
            let Ok(loc): Result<String, _> = entry.get_value("InstallLocation") else {
                continue;
            };
            let loc = loc.trim();
            if loc.is_empty() {
                continue;
            }
            let exe = PathBuf::from(loc).join(exe_filename);
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn detect_via_registry(_exe_filename: &str) -> Option<PathBuf> {
    None
}

/// 표준 설치 루트(`%LOCALAPPDATA%\Programs`, `%ProgramFiles%`, `%ProgramFiles(x86)%`)
/// 바로 아래 모든 폴더를 열거해 각각에 `exe_filename`이 있는지 확인 — 폴더 이름을
/// 미리 알 필요가 없다.
fn detect_via_standard_folders(exe_filename: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join("Programs"));
    }
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        roots.push(PathBuf::from(pf));
    }
    if let Some(pf86) = std::env::var_os("ProgramFiles(x86)") {
        roots.push(PathBuf::from(pf86));
    }
    find_exe_in_subfolders(&roots, exe_filename)
}

/// `roots` 각각의 바로 아래 폴더들을 전부 훑어 `<하위폴더>/<exe_filename>`을 확인 —
/// env var 읽기와 분리해 순수 함수로 테스트 가능하게 한다.
fn find_exe_in_subfolders(roots: &[PathBuf], exe_filename: &str) -> Option<PathBuf> {
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let exe = entry.path().join(exe_filename);
            if exe.is_file() {
                return Some(exe);
            }
        }
    }
    None
}

/// `data_root/instances_subfolder/instance_id` 안전 join. `instance_id` 가 fs-safe 라는
/// invariant 가 지켜진다고 가정 — 위반 시 debug 빌드는 panic, release 는 instances 폴더
/// 자체로 fallback(최악의 경우라도 traversal 차단). 호출자는 항상 `validate_service_id`
/// 통과한 id 만 넘겨야 한다.
pub fn instance_dir(data_root: &Path, instances_subfolder: &str, instance_id: &str) -> PathBuf {
    debug_assert!(
        crate::is_valid_service_id(instance_id),
        "instance_dir 가 unsafe id 를 받음: {instance_id:?} — 호출자가 validate_service_id 누락"
    );
    let instances = data_root.join(instances_subfolder);
    if !crate::is_valid_service_id(instance_id) {
        return instances;
    }
    instances.join(instance_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 합성(가짜) 값만 사용 — 이 모듈은 범용 로직을 테스트하는 것이지 Prism 을 테스트하는
    // 게 아니다. 실제 Prism descriptor는 코드가 아니라 데이터라 여기 테스트로 남길
    // 이유가 없다.
    fn test_descriptor() -> ThirdPartyAppDescriptor {
        ThirdPartyAppDescriptor {
            id: "test_app".to_string(),
            label: None,
            exe_filename: "test_app.exe".to_string(),
            download_strategy: None,
            instances_subfolder: Some("instances".to_string()),
            system_appdata_folder_name: None,
            readiness_signal: None,
            launch_args_template: vec![],
            post_download_marker_files: vec![],
        }
    }

    fn temp_subdir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pengport-thirdparty-test-{label}-{}-{}",
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
    fn resolve_prefers_user_override_over_bundled() {
        let override_root = temp_subdir("resolve-override");
        let bundled_root = temp_subdir("resolve-bundled");
        std::fs::write(override_root.join("test_app.exe"), b"dummy").unwrap();
        std::fs::write(bundled_root.join("test_app.exe"), b"dummy").unwrap();

        let ctx = DataRootLookupContext {
            user_override_root: Some(override_root.clone()),
            bundled_root: Some(bundled_root.clone()),
        };
        let resolved = resolve_third_party_app(&test_descriptor(), &ctx).unwrap();

        let _ = std::fs::remove_dir_all(&override_root);
        let _ = std::fs::remove_dir_all(&bundled_root);

        assert_eq!(resolved.source, ThirdPartyAppSource::UserOverride);
        assert_eq!(resolved.data_root, override_root);
    }

    #[test]
    fn resolve_falls_back_to_bundled_when_no_override() {
        let bundled_root = temp_subdir("resolve-bundled-only");
        std::fs::write(bundled_root.join("test_app.exe"), b"dummy").unwrap();

        let ctx = DataRootLookupContext {
            user_override_root: None,
            bundled_root: Some(bundled_root.clone()),
        };
        let resolved = resolve_third_party_app(&test_descriptor(), &ctx).unwrap();
        let _ = std::fs::remove_dir_all(&bundled_root);

        assert_eq!(resolved.source, ThirdPartyAppSource::Bundled);
        assert_eq!(resolved.data_root, bundled_root);
    }

    #[test]
    fn resolve_ignores_override_dir_missing_exe() {
        // override 폴더는 존재하지만 exe_filename 이 없으면 무시하고 bundled 로 폴백.
        let override_root = temp_subdir("resolve-override-empty");
        let bundled_root = temp_subdir("resolve-bundled-fallback");
        std::fs::write(bundled_root.join("test_app.exe"), b"dummy").unwrap();

        let ctx = DataRootLookupContext {
            user_override_root: Some(override_root.clone()),
            bundled_root: Some(bundled_root.clone()),
        };
        let resolved = resolve_third_party_app(&test_descriptor(), &ctx).unwrap();

        let _ = std::fs::remove_dir_all(&override_root);
        let _ = std::fs::remove_dir_all(&bundled_root);

        assert_eq!(resolved.source, ThirdPartyAppSource::Bundled);
    }

    #[test]
    fn resolve_none_when_nothing_matches() {
        let ctx = DataRootLookupContext::default();
        assert!(resolve_third_party_app(&test_descriptor(), &ctx).is_none());
    }

    #[test]
    fn detect_returns_none_when_nothing_found() {
        // 실제 시스템에 이 이름의 exe 가 설치돼 있지 않다는 전제(테스트 환경 가정).
        assert!(detect("pengport-test-nonexistent-12345.exe").is_none());
    }

    #[test]
    fn find_exe_in_subfolders_locates_match() {
        let tmp = temp_subdir("find-exe");
        let app_dir = tmp.join("TestApp");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(app_dir.join("test_app.exe"), b"dummy").unwrap();

        let found = find_exe_in_subfolders(std::slice::from_ref(&tmp), "test_app.exe");
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(found, Some(app_dir.join("test_app.exe")));
    }

    #[test]
    fn find_exe_in_subfolders_none_when_missing() {
        let tmp = temp_subdir("find-exe-missing");
        let found = find_exe_in_subfolders(std::slice::from_ref(&tmp), "app.exe");
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(found.is_none());
    }

    #[test]
    fn resolve_uses_system_detection_when_nothing_else_matches() {
        let tmp = temp_subdir("resolve-system");
        let app_dir = tmp.join("TestApp");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(app_dir.join("test_app.exe"), b"dummy").unwrap();
        // PATH 를 이 폴더 하나로 한정해서 System 탐지가 여기서 잡히게 한다.
        let old_path = std::env::var_os("PATH");
        std::env::set_var("PATH", &app_dir);

        let descriptor = ThirdPartyAppDescriptor {
            system_appdata_folder_name: Some("TestAppUniqueAppdataFolder".to_string()),
            ..test_descriptor()
        };
        let resolved = resolve_third_party_app(&descriptor, &DataRootLookupContext::default());

        if let Some(p) = old_path {
            std::env::set_var("PATH", p);
        }
        let _ = std::fs::remove_dir_all(&tmp);

        let resolved = resolved.unwrap();
        assert_eq!(resolved.source, ThirdPartyAppSource::System);
        let expected = PathBuf::from(std::env::var_os("APPDATA").unwrap()).join("TestAppUniqueAppdataFolder");
        assert_eq!(resolved.data_root, expected);
    }

    #[test]
    fn instance_dir_joins_subfolder_and_id() {
        let root = PathBuf::from("C:/fake/data-root");
        let dir = instance_dir(&root, "instances", "my-service");
        assert_eq!(dir, root.join("instances").join("my-service"));
    }

    #[test]
    fn build_launch_args_substitutes_placeholder() {
        let template = vec!["--launch".to_string(), "{instance_id}".to_string()];
        assert_eq!(
            build_launch_args(&template, "my-service"),
            vec!["--launch".to_string(), "my-service".to_string()]
        );
    }

    #[test]
    fn build_launch_args_empty_template_yields_empty_args() {
        assert!(build_launch_args(&[], "my-service").is_empty());
    }

    #[test]
    fn build_launch_args_no_placeholder_passes_through() {
        let template = vec!["--fixed-flag".to_string()];
        assert_eq!(
            build_launch_args(&template, "my-service"),
            vec!["--fixed-flag".to_string()]
        );
    }
}

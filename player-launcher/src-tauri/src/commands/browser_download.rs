//! 직접 다운로드가 안 되는 것으로 판명된(`library::download_and_verify_to_file`이
//! 응답을 받아봤더니 실제 파일이 아니라 페이지였음을 감지한) 압축의 다운로드
//! 보조 — PengPort가 URL을 fetch하지 않고 기본 브라우저로 열어준 뒤, 사람이
//! 평소처럼 받은 파일을 다운로드 폴더에서 자동으로 찾아낸다.
//!
//! 설계(자세한 배경은 `docs/track` 대신 이 세션 대화에 있음 — 요약만):
//! - PengPort는 어떤 호스팅 서비스와도 결합되지 않는다(API 키·프로토콜 구현 불필요).
//!   특정 서비스가 막히면 레시피 작성자가 URL만 바꾸면 복구된다.
//! - "어떤 파일이 그 파일인가"는 파일명이 아니라 [`ArtifactVerification`] 해시로
//!   판정한다 — 검증과 식별을 같은 메커니즘 하나로 해결.
//! - 위치 예측은 브라우저 설정 파일을 직접 읽는다(Chrome/Edge `Preferences` JSON,
//!   Firefox `prefs.js`). 이 파싱 로직은 순수 함수로 분리해 실제 브라우저 설치 없이도
//!   결정론적으로 테스트 가능하게 한다 — "판단 로직은 항상 fixture로 영구 테스트"
//!   원칙.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use notify_debouncer_full::notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use pengport_shared::library::{ArtifactVerification, Sha256Verifier};
use tauri::Manager;

use super::library::{check_cancelled, INSTALL_CANCELLED_SENTINEL};

/// 지금 이 브라우저(들)이 실제로 어디에 저장하도록 설정돼 있는지 추정한 폴더 목록과,
/// OS 기본 다운로드/바탕화면 폴더(브라우저가 "매번 물어보기"거나 설정을 못 읽은
/// 경우 대비 폴백)를 합친 것. 어느 브라우저로 열릴지 PengPort가 모르므로 설치된
/// 것들의 후보를 전부 모아 넓게 감시한다 — 후보가 늘어도 최종 판정은 해시라서 안전하다.
pub(super) fn predict_download_dirs(app: &tauri::AppHandle) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
        let local_appdata = PathBuf::from(local_appdata);
        dirs.extend(chrome_family_download_dirs(
            &local_appdata.join("Google").join("Chrome").join("User Data"),
        ));
        dirs.extend(chrome_family_download_dirs(
            &local_appdata.join("Microsoft").join("Edge").join("User Data"),
        ));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.extend(firefox_download_dirs(
            &PathBuf::from(appdata).join("Mozilla").join("Firefox"),
        ));
    }

    // OS 기본 폴백 — 브라우저 설정을 못 읽었거나 "매번 물어보기"인 경우.
    if let Ok(d) = app.path().download_dir() {
        dirs.push(d);
    }
    if let Ok(d) = app.path().desktop_dir() {
        dirs.push(d);
    }

    dirs.sort();
    dirs.dedup();
    dirs.retain(|d| d.is_dir());
    dirs
}

/// Chrome/Edge의 `User Data` 안 프로필들(`Default`, `Profile 1`, `Profile 2`...)을
/// 훑어서 각 프로필의 `Preferences`(JSON)에서 다운로드 폴더를 읽는다. I/O는 여기서만
/// 하고, 판정은 [`parse_chrome_download_dir`](순수 함수)에 위임.
fn chrome_family_download_dirs(user_data_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(user_data_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != "Default" && !name.starts_with("Profile ") {
            continue;
        }
        let prefs_path = entry.path().join("Preferences");
        let Ok(text) = std::fs::read_to_string(&prefs_path) else {
            continue;
        };
        if let Some(dir) = parse_chrome_download_dir(&text) {
            out.push(dir);
        }
    }
    out
}

/// `Preferences` JSON에서 `download.default_directory`만 뽑는 순수 함수 — 실제
/// Chrome/Edge 설치 없이도 fixture 문자열로 테스트 가능.
fn parse_chrome_download_dir(preferences_json: &str) -> Option<PathBuf> {
    let json: serde_json::Value = serde_json::from_str(preferences_json).ok()?;
    let dir = json.pointer("/download/default_directory")?.as_str()?;
    if dir.is_empty() {
        return None;
    }
    Some(PathBuf::from(dir))
}

/// Firefox `profiles.ini`로 프로필 경로들을 찾고, 각 프로필의 `prefs.js`에서 다운로드
/// 폴더를 읽는다.
fn firefox_download_dirs(firefox_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(ini_text) = std::fs::read_to_string(firefox_root.join("profiles.ini")) else {
        return out;
    };
    for rel_path in parse_profiles_ini_paths(&ini_text) {
        let Ok(prefs_text) = std::fs::read_to_string(firefox_root.join(&rel_path).join("prefs.js")) else {
            continue;
        };
        if let Some(dir) = parse_firefox_download_dir(&prefs_text) {
            out.push(dir);
        }
    }
    out
}

/// `profiles.ini`의 각 `Path=` 값을 뽑는 순수 함수(섹션 종류·`IsRelative` 무관하게
/// 전부 후보로 삼는다 — 상대/절대 경로 판별까지는 안 하고, 호출자가
/// `firefox_root.join(rel_path)`로 결합했을 때 존재하지 않으면 자연히 무시됨).
fn parse_profiles_ini_paths(ini_text: &str) -> Vec<String> {
    ini_text
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Path=").map(|p| p.trim().to_string()))
        .collect()
}

/// `prefs.js`에서 `browser.download.folderList`가 `2`(사용자 지정)일 때만
/// `browser.download.dir` 값을 반환하는 순수 함수. `0`/`1`(바탕화면/기본 다운로드
/// 폴더)이면 `None` — 그 경우는 이미 호출자가 등록하는 OS 기본 폴백과 겹치므로
/// 중복 등록하지 않는다.
fn parse_firefox_download_dir(prefs_text: &str) -> Option<PathBuf> {
    let uses_custom_dir = prefs_text
        .lines()
        .any(|l| l.contains("browser.download.folderList") && l.contains(", 2)"));
    if !uses_custom_dir {
        return None;
    }
    for line in prefs_text.lines() {
        let Some(key_idx) = line.find("user_pref(\"browser.download.dir\"") else {
            continue;
        };
        let rest = &line[key_idx..];
        let value_start = rest.find(", \"")? + 3;
        let after_start = &rest[value_start..];
        let value_end = after_start.find("\");")?;
        let raw = &after_start[..value_end];
        return Some(PathBuf::from(raw.replace("\\\\", "\\")));
    }
    None
}

/// `dirs`를 감시하다가, 새/변경된 파일 중 `verification` 해시와 일치하는 걸 찾으면
/// 그 경로를 반환한다. 파일 크기가 잠시 안정된 뒤에만 해싱하도록 debounce(2초) —
/// 다운로드 중인 대용량 파일을 매 변경 이벤트마다 통째로 재해싱하는 낭비 방지.
/// 브라우저가 임시 확장자(`.crdownload`/`.part` 등)를 쓰든 안 쓰든 상관없다 — 아직
/// 다운로드 중인 파일은 해시가 그냥 안 맞을 뿐이라 자연히 계속 감시된다.
/// 해시가 일치하는 걸 찾으면 그 자리에서 바로 `tmp_dir`로 복사하고(원본 확장자
/// 보존 — `extract_archive_file`이 형식 판별에 씀) 복사된 경로를 반환한다. "일치
/// 확인"과 "복사"를 분리하지 않고 같은 루프 이터레이션에서 바로 잇는 이유: Chrome/Edge는
/// 다운로드 완료 시점에 임시 파일명(`미확인 NNNNN.crdownload`)에 내용을 다 쓴 뒤 최종
/// 파일명으로 rename한다. 해시 검사는
/// 그 rename 직전 순간에 성공할 수 있는데, 그 뒤에(별도 함수 호출을 거쳐) 복사를
/// 시도하면 그 사이 rename이 끝나버려 원본 경로가 이미 없어져 있을 수 있다 — 검사와
/// 복사 사이 텀을 최소화해도 이 경합 자체는 원리적으로 없앨 수 없으므로, 복사가
/// "파일 없음"으로 실패하면 에러로 끝내지 않고 계속 감시한다: rename 자체가 새
/// 알림 이벤트를 발생시켜서, 최종 파일명으로 다시 나타난 같은 내용의 파일이 다음
/// 루프에서 자연히 잡힌다.
pub(super) fn watch_for_matching_file(
    dirs: &[PathBuf],
    verification: &ArtifactVerification,
    cancel_flag: &AtomicBool,
    timeout: Duration,
    tmp_dir: &Path,
) -> Result<PathBuf, String> {
    use std::sync::mpsc::{channel, RecvTimeoutError};

    if dirs.is_empty() {
        return Err("감시할 다운로드 폴더를 찾지 못했습니다".to_string());
    }

    let (tx, rx) = channel();
    let mut debouncer = new_debouncer(Duration::from_secs(2), None, move |res| {
        let _ = tx.send(res);
    })
    .map_err(|e| format!("다운로드 폴더 감시 시작 실패: {e}"))?;
    for dir in dirs {
        // 폴더 하나가 없어졌거나(외장 드라이브 분리 등) 권한 문제여도 나머지 감시는
        // 계속되도록 개별 실패를 무시한다.
        let _ = debouncer.watch(dir, RecursiveMode::NonRecursive);
    }

    let deadline = Instant::now() + timeout;
    loop {
        check_cancelled(cancel_flag)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(
                "다운로드 대기 시간 초과 — 브라우저에서 받은 파일을 찾지 못했습니다".to_string(),
            );
        }
        match rx.recv_timeout(remaining.min(Duration::from_millis(500))) {
            Ok(Ok(events)) => {
                for event in events {
                    for path in &event.paths {
                        if !path.is_file() || !file_matches(path, verification) {
                            continue;
                        }
                        match copy_matched_file(path, tmp_dir) {
                            Ok(tmp_path) => return Ok(tmp_path),
                            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                // 위 doc comment의 rename 경합 — 이 후보는 놓쳤지만
                                // 최종 파일명으로 곧 다시 나타날 테니 계속 감시.
                                continue;
                            }
                            Err(e) => {
                                return Err(format!(
                                    "받은 파일 복사 실패 ({} → {}): {e}",
                                    path.display(),
                                    tmp_dir.display()
                                ));
                            }
                        }
                    }
                }
            }
            Ok(Err(_)) => continue,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err(INSTALL_CANCELLED_SENTINEL.to_string());
            }
        }
    }
}

fn file_matches(path: &Path, verification: &ArtifactVerification) -> bool {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut hasher = Sha256Verifier::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return false,
        }
    }
    hasher.finish(verification).is_ok()
}

/// `matched`를 `tmp_dir` 안으로 복사 — `download_and_verify_to_file`의 직다운로드
/// 경로는 항상 확장자 없는 "download.part"를 쓰지만(형식 판별을 URL 확장자로 하니까),
/// 여기서는 반대로 실제 URL에 확장자가 없을 수 있으므로(다운로드 페이지 링크) 받은
/// 파일 자신의 확장자를 그대로 살린다(`extract_format_hint`가 이 경로로 형식 판별).
fn copy_matched_file(matched: &Path, tmp_dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(tmp_dir)?;
    let tmp_filename = match matched.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("download.{ext}"),
        None => "download.part".to_string(),
    };
    let tmp_path = tmp_dir.join(tmp_filename);
    std::fs::copy(matched, &tmp_path)?;
    Ok(tmp_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pengport-browser-download-test-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn copy_matched_file_preserves_extension() {
        let src_dir = temp_test_dir("copy-src-ext");
        std::fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("DPJAM_skin.zip");
        std::fs::write(&src, b"fake zip bytes").unwrap();

        let tmp_dir = temp_test_dir("copy-dst-ext");
        let copied = copy_matched_file(&src, &tmp_dir).unwrap();

        assert_eq!(copied, tmp_dir.join("download.zip"));
        assert_eq!(std::fs::read(&copied).unwrap(), b"fake zip bytes");

        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn copy_matched_file_falls_back_to_download_part_when_no_extension() {
        let src_dir = temp_test_dir("copy-src-noext");
        std::fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("noext");
        std::fs::write(&src, b"x").unwrap();

        let tmp_dir = temp_test_dir("copy-dst-noext");
        let copied = copy_matched_file(&src, &tmp_dir).unwrap();

        assert_eq!(copied, tmp_dir.join("download.part"));

        let _ = std::fs::remove_dir_all(&src_dir);
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn copy_matched_file_returns_not_found_when_source_vanished() {
        // 호출자는 이 에러 종류(NotFound)를 "치명적 에러"가 아니라 "계속 감시"로 분기한다.
        let src_dir = temp_test_dir("copy-src-vanished");
        let src = src_dir.join("미확인 282412.crdownload"); // 존재하지 않음(폴더조차 안 만듦)
        let tmp_dir = temp_test_dir("copy-dst-vanished");

        let err = copy_matched_file(&src, &tmp_dir).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn parse_chrome_download_dir_reads_default_directory() {
        let json = r#"{"download": {"default_directory": "D:\\Downloads"}, "other": 1}"#;
        assert_eq!(
            parse_chrome_download_dir(json),
            Some(PathBuf::from("D:\\Downloads"))
        );
    }

    #[test]
    fn parse_chrome_download_dir_none_when_missing() {
        assert_eq!(parse_chrome_download_dir(r#"{"other": 1}"#), None);
    }

    #[test]
    fn parse_chrome_download_dir_none_when_empty_string() {
        let json = r#"{"download": {"default_directory": ""}}"#;
        assert_eq!(parse_chrome_download_dir(json), None);
    }

    #[test]
    fn parse_chrome_download_dir_none_when_malformed_json() {
        assert_eq!(parse_chrome_download_dir("not json"), None);
    }

    #[test]
    fn parse_profiles_ini_paths_extracts_all_path_lines() {
        let ini = "[Profile0]\nName=default\nIsRelative=1\nPath=xxxxxxxx.default-release\n\n[Profile1]\nPath=yyyyyyyy.dev-edition-default\n";
        assert_eq!(
            parse_profiles_ini_paths(ini),
            vec!["xxxxxxxx.default-release", "yyyyyyyy.dev-edition-default"]
        );
    }

    #[test]
    fn parse_profiles_ini_paths_empty_when_no_sections() {
        assert!(parse_profiles_ini_paths("").is_empty());
    }

    #[test]
    fn parse_firefox_download_dir_reads_custom_dir_when_folder_list_is_2() {
        let prefs = "user_pref(\"browser.download.folderList\", 2);\nuser_pref(\"browser.download.dir\", \"C:\\\\Users\\\\me\\\\Desktop\\\\dl\");\n";
        assert_eq!(
            parse_firefox_download_dir(prefs),
            Some(PathBuf::from("C:\\Users\\me\\Desktop\\dl"))
        );
    }

    #[test]
    fn parse_firefox_download_dir_none_when_folder_list_is_default() {
        // folderList=1(기본 다운로드 폴더) — OS 기본 폴백과 중복 등록하지 않아야 함.
        let prefs = "user_pref(\"browser.download.folderList\", 1);\nuser_pref(\"browser.download.dir\", \"C:\\\\stale\\\\value\");\n";
        assert_eq!(parse_firefox_download_dir(prefs), None);
    }

    #[test]
    fn parse_firefox_download_dir_none_when_dir_pref_missing() {
        let prefs = "user_pref(\"browser.download.folderList\", 2);\n";
        assert_eq!(parse_firefox_download_dir(prefs), None);
    }
}

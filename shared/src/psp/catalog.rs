//! services.toml / services.d/ catalog schema.
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 4.
//!
//! ## 두 가지 운영자 형식
//!
//! 1. **단일 파일** (`services.toml`) — 전체 catalog 한 파일. 작은 인스턴스용.
//! 2. **디렉토리** (`services.d/*.toml`) — service 별 별도 파일. Linux `*.d` 패턴.
//!
//! 클라이언트는 항상 **단일 catalog 응답** 만 받는다. gateway (또는 정적 호스팅 도구)
//! 가 디렉토리 모드 시 합쳐서 응답. `merge_catalog_dir()` 가 그 통합 helper.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 카탈로그 응답 (단일 services.toml 또는 합쳐진 services.d/).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServicesCatalog {
    /// 항상 `"2"`. PSP v1 에선 v1 catalog (servers.toml) 를 backward compat.
    pub schema_version: String,

    #[serde(default)]
    pub instance: Option<InstanceInfo>,

    #[serde(default)]
    pub services: Vec<ServiceEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstanceInfo {
    pub display_name: String,

    #[serde(default)]
    pub description: Option<String>,
}

/// services.toml 의 한 `[[services]]` 항목. 자세한 정보는 `manifest` URL 호출
/// 후 받음.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceEntry {
    pub id: String,

    /// 항상 `"psp"`. 미래에 다른 type 추가 가능 (현재는 PSP 만).
    #[serde(rename = "type")]
    pub kind: String,

    /// service base URL. 클라이언트가 `{url}/.well-known/pengport-service` 호출.
    pub url: String,

    #[serde(default = "default_enabled")]
    pub enabled: bool,

    #[serde(default)]
    pub hint: Option<ServiceHint>,
}

fn default_enabled() -> bool {
    true
}

/// manifest fetch 전 클라이언트가 보여주기 위한 hint (선택).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceHint {
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Debug, Error)]
pub enum CatalogMergeError {
    #[error("디렉토리 읽기 실패 ({path}): {source}")]
    DirRead {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("파일 읽기 실패 ({path}): {source}")]
    FileRead {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("toml 파싱 실패 ({path}): {source}")]
    Parse {
        path: String,
        #[source]
        source: toml::de::Error,
    },

    #[error("중복 service id '{id}' (파일: {first} / {second})")]
    DuplicateId {
        id: String,
        first: String,
        second: String,
    },

    #[error("schema_version 불일치: '{found}' (디렉토리 모드는 모든 파일이 schema_version 일치 필요)")]
    SchemaMismatch { found: String },
}

/// 한 디렉토리 안의 `*.toml` 을 모두 읽어 단일 `ServicesCatalog` 으로 합친다.
///
/// 운영자 측 gateway 가 호출 (`/services` GET 응답 생성). 클라이언트는
/// 항상 단일 응답만 받는다.
///
/// ## 규칙
///
/// - 각 파일은 부분 catalog (header + `[[services]]` 섹션) — `parse_partial` 로 파싱
/// - `instance` 헤더는 **첫 파일** 만 채택. 나머지 파일의 `[instance]` 는 무시 (경고 없음)
/// - 모든 파일의 `schema_version` 동일해야 함
/// - 같은 `service.id` 가 둘 이상 나타나면 에러 (운영자가 의도치 않은 중복 방지)
/// - `_` 로 시작하는 파일은 skip (예: `_template.toml`)
pub fn merge_catalog_dir(dir: &Path) -> Result<ServicesCatalog, CatalogMergeError> {
    let entries = fs::read_dir(dir).map_err(|e| CatalogMergeError::DirRead {
        path: dir.display().to_string(),
        source: e,
    })?;

    let mut paths: Vec<std::path::PathBuf> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".toml") && !name.starts_with('_')
        })
        .map(|e| e.path())
        .collect();
    paths.sort();

    let mut merged: Option<ServicesCatalog> = None;
    let mut seen_ids: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for path in &paths {
        let text = fs::read_to_string(path).map_err(|e| CatalogMergeError::FileRead {
            path: path.display().to_string(),
            source: e,
        })?;
        let part: ServicesCatalog =
            toml::from_str(&text).map_err(|e| CatalogMergeError::Parse {
                path: path.display().to_string(),
                source: e,
            })?;

        match merged.as_mut() {
            None => {
                merged = Some(part);
            }
            Some(m) => {
                if m.schema_version != part.schema_version {
                    return Err(CatalogMergeError::SchemaMismatch {
                        found: part.schema_version,
                    });
                }
                for s in part.services {
                    if let Some(prev) = seen_ids.get(&s.id) {
                        return Err(CatalogMergeError::DuplicateId {
                            id: s.id.clone(),
                            first: prev.clone(),
                            second: path.display().to_string(),
                        });
                    }
                    seen_ids.insert(s.id.clone(), path.display().to_string());
                    m.services.push(s);
                }
            }
        }

        // 첫 파일의 service 들도 dedup tracking
        if let Some(m) = merged.as_ref() {
            if seen_ids.is_empty() {
                for s in &m.services {
                    seen_ids.insert(s.id.clone(), path.display().to_string());
                }
            }
        }
    }

    Ok(merged.unwrap_or_else(|| ServicesCatalog {
        schema_version: "2".to_string(),
        instance: None,
        services: Vec::new(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_catalog_toml() {
        let toml_str = r#"
            schema_version = "2"

            [[services]]
            id = "test"
            type = "psp"
            url = "https://x.example"
        "#;
        let c: ServicesCatalog = toml::from_str(toml_str).unwrap();
        assert_eq!(c.schema_version, "2");
        assert_eq!(c.services.len(), 1);
        assert_eq!(c.services[0].id, "test");
        assert!(c.services[0].enabled); // default
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir()
            .join(format!("pengport-catalog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn merge_dir_combines_files() {
        let dir = temp_dir("merge-combine");
        std::fs::write(
            dir.join("modded.toml"),
            r#"schema_version = "2"
[instance]
display_name = "펭돌서버"

[[services]]
id = "modded-mc"
type = "psp"
url = "https://mc.example/modded""#,
        )
        .unwrap();
        std::fs::write(
            dir.join("rlcraft.toml"),
            r#"schema_version = "2"

[[services]]
id = "rlcraft-mc"
type = "psp"
url = "https://mc.example/rlcraft""#,
        )
        .unwrap();

        let merged = merge_catalog_dir(&dir).unwrap();
        assert_eq!(merged.services.len(), 2);
        assert!(merged.services.iter().any(|s| s.id == "modded-mc"));
        assert!(merged.services.iter().any(|s| s.id == "rlcraft-mc"));
        assert_eq!(
            merged.instance.as_ref().unwrap().display_name,
            "펭돌서버"
        );
    }

    #[test]
    fn merge_dir_rejects_duplicate_ids() {
        let dir = temp_dir("merge-dup");
        std::fs::write(
            dir.join("a.toml"),
            r#"schema_version = "2"
[[services]]
id = "x"
type = "psp"
url = "https://a""#,
        )
        .unwrap();
        std::fs::write(
            dir.join("b.toml"),
            r#"schema_version = "2"
[[services]]
id = "x"
type = "psp"
url = "https://b""#,
        )
        .unwrap();

        match merge_catalog_dir(&dir) {
            Err(CatalogMergeError::DuplicateId { id, .. }) => assert_eq!(id, "x"),
            other => panic!("expected DuplicateId, got {other:?}"),
        }
    }

    #[test]
    fn merge_dir_skips_underscore_files() {
        let dir = temp_dir("merge-underscore");
        std::fs::write(
            dir.join("_template.toml"),
            r#"schema_version = "2"
[[services]]
id = "template-only"
type = "psp"
url = "https://nope""#,
        )
        .unwrap();
        std::fs::write(
            dir.join("real.toml"),
            r#"schema_version = "2"
[[services]]
id = "real"
type = "psp"
url = "https://real""#,
        )
        .unwrap();

        let merged = merge_catalog_dir(&dir).unwrap();
        assert_eq!(merged.services.len(), 1);
        assert_eq!(merged.services[0].id, "real");
    }

    #[test]
    fn merge_dir_empty_returns_empty_catalog() {
        let dir = temp_dir("merge-empty");
        let merged = merge_catalog_dir(&dir).unwrap();
        assert_eq!(merged.schema_version, "2");
        assert!(merged.services.is_empty());
    }

    #[test]
    fn merge_dir_rejects_schema_mismatch() {
        let dir = temp_dir("merge-mismatch");
        std::fs::write(
            dir.join("v2.toml"),
            r#"schema_version = "2"
[[services]]
id = "a"
type = "psp"
url = "https://a""#,
        )
        .unwrap();
        std::fs::write(
            dir.join("v3.toml"),
            r#"schema_version = "3"
[[services]]
id = "b"
type = "psp"
url = "https://b""#,
        )
        .unwrap();

        match merge_catalog_dir(&dir) {
            Err(CatalogMergeError::SchemaMismatch { found }) => assert_eq!(found, "3"),
            other => panic!("expected SchemaMismatch, got {other:?}"),
        }
    }

    #[test]
    fn parse_full_catalog_toml() {
        let toml_str = r#"
            schema_version = "2"

            [instance]
            display_name = "펭돌서버"
            description = "친구 그룹용"

            [[services]]
            id = "modded-mc"
            type = "psp"
            url = "https://mc-adapter.x.example"
            enabled = true

              [services.hint]
              name = "알파펭"
              icon = "https://x.example/icon.png"

            [[services]]
            id = "rlcraft-mc"
            type = "psp"
            url = "https://mc-rlcraft.x.example"
            enabled = false
        "#;
        let c: ServicesCatalog = toml::from_str(toml_str).unwrap();
        assert_eq!(c.instance.as_ref().unwrap().display_name, "펭돌서버");
        assert_eq!(c.services.len(), 2);
        assert_eq!(c.services[0].hint.as_ref().unwrap().name.as_deref(), Some("알파펭"));
        assert!(!c.services[1].enabled);
    }
}

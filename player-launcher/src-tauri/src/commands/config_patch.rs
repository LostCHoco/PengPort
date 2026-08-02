//! [`OverrideContent::ConfigPatch`](pengport_shared::library::OverrideContent)의 실제
//! 파일 반영 — 포맷 무관 공통 진입점.
//!
//! 레시피의 `patch`는 항상 `serde_json::Value`(중첩 객체) — 포맷별로 다른 것은
//! "그 포맷의 텍스트를 어떻게 이 JSON 모양으로 읽고/이 JSON 모양을 어떻게 그 텍스트에
//! 반영하는가" 뿐이다. 새 포맷 추가 = [`apply_config_patch`]/[`parse_to_json`]에 분기
//! 하나씩 추가 — 레시피 스키마도 프론트 UI 코드도 그대로.
//!
//! ini만 "기존 내용 보존 patch"(다른 section/key/주석 안 건드림)이고, json/toml은
//! "전체를 파싱해서 patch를 재귀 병합 후 다시 직렬화"다 — ini는 텍스트 기반 patch가
//! 아니면 사람이 직접 편집해둔 포맷 스타일이 깨지기 쉬운 반면, json/toml 직렬화는
//! 결과가 기능적으로 동등하면 충분하다고 판단(포맷 관례 차이).
//!
//! **설치 상태 판정용 "실제 파일과 비교" 함수(옛 `patch_matches`/`diff_config_patch`)는
//! 의도적으로 없다** — `commands/library.rs`가 원장(마커) 기반으로만 판정한다(모듈
//! 설명 참고). 이 파일은 순수하게 "patch를 어떻게 적용/파싱하는가"만 담당한다.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use pengport_shared::library::ConfigFileFormat;
use serde_json::Value;

/// `path`(이미 존재해야 함)에 `patch`를 반영 — 기존 내용은 최대한 보존.
pub fn apply_config_patch(format: ConfigFileFormat, path: &Path, patch: &Value) -> Result<(), String> {
    let original = fs::read_to_string(path)
        .map_err(|e| format!("설정 파일 읽기 실패 ({}): {e}", path.display()))?;
    let updated = match format {
        ConfigFileFormat::Ini => apply_ini_patch(&original, patch)?,
        ConfigFileFormat::Json => apply_json_patch(&original, patch)?,
        ConfigFileFormat::Toml => apply_toml_patch(&original, patch)?,
    };
    fs::write(path, updated).map_err(|e| format!("설정 파일 쓰기 실패 ({}): {e}", path.display()))
}

/// 레시피 편집 화면의 "파일에서 불러오기" — 기존 파일 전체를 `patch`와 같은 JSON
/// 모양으로 파싱(부분이 아니라 전부, `apply_config_patch`의 patch 방향과 반대).
pub fn parse_to_json(format: ConfigFileFormat, text: &str) -> Result<Value, String> {
    match format {
        ConfigFileFormat::Ini => Ok(parse_ini_to_json(text)),
        ConfigFileFormat::Json => {
            serde_json::from_str(text).map_err(|e| format!("JSON 파싱 실패: {e}"))
        }
        ConfigFileFormat::Toml => {
            let v: toml::Value = toml::from_str(text).map_err(|e| format!("TOML 파싱 실패: {e}"))?;
            serde_json::to_value(v).map_err(|e| format!("TOML→JSON 변환 실패: {e}"))
        }
    }
}

// ---------------------------------------------------------------------------
// JSON — patch 자체가 이미 목표 모양이라 재귀 병합만 하면 됨.
// ---------------------------------------------------------------------------

fn apply_json_patch(original: &str, patch: &Value) -> Result<String, String> {
    let mut base: Value = if original.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(original).map_err(|e| format!("JSON 파싱 실패: {e}"))?
    };
    merge_json(&mut base, patch);
    serde_json::to_string_pretty(&base).map_err(|e| format!("JSON 직렬화 실패: {e}"))
}

/// `patch`를 `base`에 재귀 병합 — object는 key별로 파고들고, 그 외(문자열/숫자/배열 등)는
/// patch 값으로 완전히 대체. json/toml 둘 다 이 함수 하나를 공유(toml 은 Value 를
/// serde_json::Value 로 변환한 뒤 병합 — 병합 알고리즘 자체는 포맷 무관).
fn merge_json(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (key, patch_val) in patch_map {
                merge_json(
                    base_map.entry(key.clone()).or_insert(Value::Null),
                    patch_val,
                );
            }
        }
        (base_slot, patch_val) => {
            *base_slot = patch_val.clone();
        }
    }
}

// ---------------------------------------------------------------------------
// TOML — toml::Value ↔ serde_json::Value 변환(둘 다 Serialize/Deserialize) 후
// merge_json 재사용.
// ---------------------------------------------------------------------------

fn apply_toml_patch(original: &str, patch: &Value) -> Result<String, String> {
    let base_toml: toml::Value = if original.trim().is_empty() {
        toml::Value::Table(Default::default())
    } else {
        toml::from_str(original).map_err(|e| format!("TOML 파싱 실패: {e}"))?
    };
    let mut base_json = serde_json::to_value(&base_toml).map_err(|e| format!("TOML→JSON 변환 실패: {e}"))?;
    merge_json(&mut base_json, patch);
    let merged_toml: toml::Value =
        serde_json::from_value(base_json).map_err(|e| format!("JSON→TOML 변환 실패: {e}"))?;
    toml::to_string_pretty(&merged_toml).map_err(|e| format!("TOML 직렬화 실패: {e}"))
}

// ---------------------------------------------------------------------------
// INI — 기존 라인 구조/주석/포맷을 보존하는 patch(json/toml처럼 전체 재직렬화 아님).
// `patch` 는 `{"섹션": {"키": 값}}` 모양이어야 함.
// ---------------------------------------------------------------------------

/// (section, key, value) 트리플 — ini patch 를 flatten 한 결과.
type IniEntry = (String, String, String);

fn apply_ini_patch(original: &str, patch: &Value) -> Result<String, String> {
    let entries = flatten_ini_patch(patch)?;
    if entries.is_empty() {
        return Ok(original.to_string());
    }

    let mut lines: Vec<String> = original.lines().map(str::to_string).collect();
    let mut remaining: Vec<&IniEntry> = entries.iter().collect();

    // section 이름 → 그 section 에 속한 마지막 줄의 다음 인덱스(기존 key 치환과 동시에 갱신).
    let mut section_end: HashMap<String, usize> = HashMap::new();
    let mut current_section: Option<String> = None;
    for (idx, line) in lines.clone().iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.len() >= 2 && trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = Some(trimmed[1..trimmed.len() - 1].to_string());
        }
        if let Some(sec) = &current_section {
            section_end.insert(sec.clone(), idx + 1);
        }
        if let (Some(sec), Some(eq)) = (&current_section, trimmed.find('=')) {
            let key = trimmed[..eq].trim();
            if let Some(pos) = remaining.iter().position(|(s, k, _)| s == sec && k == key) {
                let (_, k, v) = remaining.remove(pos);
                lines[idx] = format!("{k}={v}");
            }
        }
    }

    // 못 채운 entry — section 끝(또는 새 section 이면 파일 끝)에 삽입. 뒤쪽 인덱스부터
    // 처리해야 앞선 삽입이 뒤 인덱스를 밀어내지 않는다.
    let mut by_section: BTreeMap<String, Vec<&IniEntry>> = BTreeMap::new();
    for e in remaining {
        by_section.entry(e.0.clone()).or_default().push(e);
    }
    let mut inserts: Vec<(usize, String, Vec<&IniEntry>)> = by_section
        .into_iter()
        .map(|(sec, es)| {
            let at = section_end.get(&sec).copied().unwrap_or(lines.len());
            (at, sec, es)
        })
        .collect();
    inserts.sort_by(|a, b| b.0.cmp(&a.0));

    for (at, sec, es) in inserts {
        let is_new_section = !section_end.contains_key(&sec);
        let mut insert_lines = Vec::new();
        if is_new_section {
            insert_lines.push(format!("[{sec}]"));
        }
        insert_lines.extend(es.into_iter().map(|(_, k, v)| format!("{k}={v}")));
        for (offset, l) in insert_lines.into_iter().enumerate() {
            lines.insert(at + offset, l);
        }
    }

    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    Ok(out)
}

/// `{"섹션": {"키": 값}}` → `(섹션, 키, 값-문자열)` 목록. ini 값은 항상 텍스트라
/// JSON 원시값을 텍스트로 정규화(문자열은 그대로, 그 외엔 자연스러운 텍스트 표현).
fn flatten_ini_patch(patch: &Value) -> Result<Vec<IniEntry>, String> {
    let Value::Object(sections) = patch else {
        return Err("ini patch 형식 오류: 최상위는 {섹션: {키: 값}} 객체여야 함".to_string());
    };
    let mut out = Vec::new();
    for (section, keys) in sections {
        let Value::Object(keys) = keys else {
            return Err(format!("ini patch 형식 오류: 섹션 '{section}' 의 값이 객체가 아님"));
        };
        for (key, value) in keys {
            out.push((section.clone(), key.clone(), json_scalar_to_text(value)));
        }
    }
    Ok(out)
}

/// JSON 원시값을 ini 값 텍스트로 정규화 — 문자열은 그대로, 그 외(숫자·불리언 등)는
/// 자연스러운 텍스트 표현.
fn json_scalar_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn parse_ini_to_json(text: &str) -> Value {
    let mut sections = serde_json::Map::new();
    let mut current_section: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.len() >= 2 && trimmed.starts_with('[') && trimmed.ends_with(']') {
            let name = trimmed[1..trimmed.len() - 1].to_string();
            sections
                .entry(name.clone())
                .or_insert_with(|| Value::Object(Default::default()));
            current_section = Some(name);
            continue;
        }
        let Some(eq) = trimmed.find('=') else { continue };
        let key = trimmed[..eq].trim().to_string();
        let value = trimmed[eq + 1..].trim().to_string();
        let section_name = current_section.clone().unwrap_or_default();
        if let Value::Object(map) = sections
            .entry(section_name)
            .or_insert_with(|| Value::Object(Default::default()))
        {
            map.insert(key, Value::String(value));
        }
    }
    Value::Object(sections)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(ext: &str, name: &str, content: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir()
            .join(format!("pengport-config-test-{name}-{}.{ext}", std::process::id()));
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        p
    }

    // --- ini ---

    #[test]
    fn ini_replaces_existing_key_preserves_rest() {
        let path = temp_file("ini", "replace", "[GRAPHICS]\n3D_Mode=3\nEQ=1\n[Sound]\nBGVolume=255\n");
        let patch = serde_json::json!({ "GRAPHICS": { "3D_Mode": "0" } });
        apply_config_patch(ConfigFileFormat::Ini, &path, &patch).unwrap();
        let out = fs::read_to_string(&path).unwrap();
        assert!(out.contains("3D_Mode=0"));
        assert!(out.contains("EQ=1"));
        assert!(out.contains("BGVolume=255"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn ini_appends_new_key_to_existing_section() {
        let path = temp_file("ini", "append", "[GRAPHICS]\n3D_Mode=3\n[Sound]\nBGVolume=255\n");
        let patch = serde_json::json!({ "GRAPHICS": { "NewKey": "42" } });
        apply_config_patch(ConfigFileFormat::Ini, &path, &patch).unwrap();
        let out = fs::read_to_string(&path).unwrap();
        let graphics_idx = out.find("[GRAPHICS]").unwrap();
        let sound_idx = out.find("[Sound]").unwrap();
        let newkey_idx = out.find("NewKey=42").unwrap();
        assert!(graphics_idx < newkey_idx && newkey_idx < sound_idx);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn ini_creates_new_section_when_missing() {
        let path = temp_file("ini", "new-section", "[GRAPHICS]\n3D_Mode=3\n");
        let patch = serde_json::json!({ "NewSection": { "Key": "v" } });
        apply_config_patch(ConfigFileFormat::Ini, &path, &patch).unwrap();
        let out = fs::read_to_string(&path).unwrap();
        assert!(out.contains("[NewSection]"));
        assert!(out.contains("Key=v"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn ini_empty_patch_leaves_file_untouched() {
        let path = temp_file("ini", "noop", "[GRAPHICS]\n3D_Mode=3\n");
        let before = fs::read_to_string(&path).unwrap();
        apply_config_patch(ConfigFileFormat::Ini, &path, &serde_json::json!({})).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(before, after);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn ini_missing_file_is_error() {
        let path = std::env::temp_dir().join("pengport-config-test-does-not-exist.ini");
        let _ = fs::remove_file(&path);
        let err = apply_config_patch(ConfigFileFormat::Ini, &path, &serde_json::json!({"A": {"b": "c"}}))
            .unwrap_err();
        assert!(err.contains("읽기 실패"));
    }

    #[test]
    fn ini_parse_to_json_roundtrip() {
        let text = "[GRAPHICS]\n3D_Mode=3\nEQ=1\n[Sound]\nBGVolume=255\n";
        let parsed = parse_to_json(ConfigFileFormat::Ini, text).unwrap();
        assert_eq!(parsed["GRAPHICS"]["3D_Mode"], "3");
        assert_eq!(parsed["Sound"]["BGVolume"], "255");
    }

    // --- json ---

    #[test]
    fn json_merges_patch_preserving_untouched_keys() {
        let path = temp_file("json", "merge", r#"{"a": 1, "nested": {"x": 1, "y": 2}}"#);
        let patch = serde_json::json!({ "nested": { "x": 99 } });
        apply_config_patch(ConfigFileFormat::Json, &path, &patch).unwrap();
        let out: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(out["a"], 1);
        assert_eq!(out["nested"]["x"], 99);
        assert_eq!(out["nested"]["y"], 2);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn json_parse_to_json_is_identity() {
        let text = r#"{"a": {"b": 1}}"#;
        let parsed = parse_to_json(ConfigFileFormat::Json, text).unwrap();
        assert_eq!(parsed["a"]["b"], 1);
    }

    // --- toml ---

    #[test]
    fn toml_merges_patch_preserving_untouched_keys() {
        let path = temp_file("toml", "merge", "a = 1\n\n[nested]\nx = 1\ny = 2\n");
        let patch = serde_json::json!({ "nested": { "x": 99 } });
        apply_config_patch(ConfigFileFormat::Toml, &path, &patch).unwrap();
        let out: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(out["a"].as_integer(), Some(1));
        assert_eq!(out["nested"]["x"].as_integer(), Some(99));
        assert_eq!(out["nested"]["y"].as_integer(), Some(2));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn toml_parse_to_json_roundtrip() {
        let text = "[nested]\nx = 1\n";
        let parsed = parse_to_json(ConfigFileFormat::Toml, text).unwrap();
        assert_eq!(parsed["nested"]["x"], 1);
    }
}

//! Minecraft `servers.dat` (NBT) 자동 등록.
//!
//! Prism 인스턴스의 `.minecraft/servers.dat` 에 펭돌서버 entry 를 upsert 한다.
//! 사용자가 직접 추가한 다른 서버 entry 는 보존하고, 펭돌서버 entry (host:port 매칭)
//! 만 갱신/추가한다.
//!
//! 동작 보장:
//! - 사용자가 펭돌서버를 일부러 지웠어도 다음 sync 때 다시 추가됨 (= 항상 등록 유지)
//! - 사용자가 만든 다른 서버 entry 는 절대 건드리지 않음
//! - 변경 사항이 없으면 파일을 다시 쓰지 않음 (mtime 보존)
//!
//! NBT 포맷:
//! ```text
//! TAG_Compound("") {
//!     TAG_List("servers", TAG_Compound) [
//!         { name: "...", ip: "host:port", ... },
//!     ]
//! }
//! ```
//! Minecraft 의 servers.dat 은 비압축 NBT 라 fastnbt 의 from_bytes/to_bytes 로 직접 처리.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServersDatError {
    #[error("I/O 실패: {0}")]
    Io(#[from] std::io::Error),

    #[error("NBT 파싱 실패: {0}")]
    NbtRead(#[from] fastnbt::error::Error),
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ServersDat {
    /// Minecraft 가 인식하는 서버 entry 목록. 화면 위에서부터 이 순서대로 표시.
    #[serde(default)]
    pub servers: Vec<ServerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEntry {
    pub name: String,
    pub ip: String,

    /// base64 인코딩 PNG 64x64. 비워두면 Minecraft 가 첫 ping 시 favicon 자동 표시.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub icon: Option<String>,

    /// 0=프롬프트(default) / 1=수락 / 2=거부.
    #[serde(rename = "acceptTextures", skip_serializing_if = "Option::is_none", default)]
    pub accept_textures: Option<i8>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hidden: Option<i8>,
}

/// `<host>:<port>` 형식의 ip 문자열을 만든다.
/// Minecraft 는 default port (25565) 면 `:25565` 생략을 허용하지만,
/// 우리는 명시적으로 항상 포함시켜 비교를 단순화.
pub fn format_ip(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

/// `dat_path` 의 servers.dat 에서 host:port 일치 entry 를 갱신하거나, 없으면 맨 앞에 추가.
/// 변경 사항이 없으면 파일을 다시 쓰지 않는다.
///
/// 반환:
/// - `Ok(true)`  → 변경 발생 (파일이 새로 만들어졌거나 갱신됨)
/// - `Ok(false)` → 이미 같은 상태라 변경 없음
pub fn upsert_server(
    dat_path: &Path,
    name: &str,
    host: &str,
    port: u16,
) -> Result<bool, ServersDatError> {
    let target_ip = format_ip(host, port);

    let mut data: ServersDat = if dat_path.is_file() {
        let bytes = fs::read(dat_path)?;
        // 빈 파일이면 default 로 시작 (Minecraft 가 만든 후 비워둔 경우).
        if bytes.is_empty() {
            ServersDat::default()
        } else {
            fastnbt::from_bytes(&bytes)?
        }
    } else {
        ServersDat::default()
    };

    // 기존 entry 검색
    let changed = if let Some(existing) = data.servers.iter_mut().find(|s| s.ip == target_ip) {
        if existing.name != name {
            existing.name = name.to_string();
            true
        } else {
            false
        }
    } else {
        // 맨 앞에 새 entry 삽입 (사용자에게 가장 잘 보이는 위치).
        data.servers.insert(
            0,
            ServerEntry {
                name: name.to_string(),
                ip: target_ip,
                icon: None,
                accept_textures: None,
                hidden: None,
            },
        );
        true
    };

    if !changed {
        return Ok(false);
    }

    if let Some(parent) = dat_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = fastnbt::to_bytes(&data).map_err(ServersDatError::NbtRead)?;
    fs::write(dat_path, bytes)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn roundtrip_empty_file_creates_entry() {
        let tmp = tempdir_path("roundtrip");
        let dat = tmp.join("servers.dat");

        let changed = upsert_server(&dat, "AlphaPeng", "pengdoll.duckdns.org", 25566).unwrap();
        assert!(changed);

        let bytes = fs::read(&dat).unwrap();
        let parsed: ServersDat = fastnbt::from_bytes(&bytes).unwrap();
        assert_eq!(parsed.servers.len(), 1);
        assert_eq!(parsed.servers[0].name, "AlphaPeng");
        assert_eq!(parsed.servers[0].ip, "pengdoll.duckdns.org:25566");

        // 같은 상태로 다시 호출 → 변경 없음
        let changed = upsert_server(&dat, "AlphaPeng", "pengdoll.duckdns.org", 25566).unwrap();
        assert!(!changed);
    }

    #[test]
    fn preserves_other_entries_and_updates_name() {
        let tmp = tempdir_path("preserves");
        let dat = tmp.join("servers.dat");

        // 사용자가 다른 서버 entry 를 가진 dat 파일을 시뮬레이션
        let initial = ServersDat {
            servers: vec![
                ServerEntry {
                    name: "친구 서버".into(),
                    ip: "friend.example.com:25565".into(),
                    icon: None,
                    accept_textures: None,
                    hidden: None,
                },
                ServerEntry {
                    name: "Modded Survival".into(), // 옛날 이름
                    ip: "pengdoll.duckdns.org:25566".into(),
                    icon: None,
                    accept_textures: None,
                    hidden: None,
                },
            ],
        };
        let mut f = fs::File::create(&dat).unwrap();
        f.write_all(&fastnbt::to_bytes(&initial).unwrap()).unwrap();
        drop(f);

        // 새 이름 (AlphaPeng) 으로 upsert → 기존 entry 의 name 만 갱신
        let changed = upsert_server(&dat, "AlphaPeng", "pengdoll.duckdns.org", 25566).unwrap();
        assert!(changed);

        let parsed: ServersDat = fastnbt::from_bytes(&fs::read(&dat).unwrap()).unwrap();
        assert_eq!(parsed.servers.len(), 2, "다른 entry 보존돼야 함");
        // 이름 갱신된 entry 찾기
        let mine = parsed
            .servers
            .iter()
            .find(|s| s.ip == "pengdoll.duckdns.org:25566")
            .unwrap();
        assert_eq!(mine.name, "AlphaPeng");
        // 친구 서버 보존
        assert!(parsed.servers.iter().any(|s| s.name == "친구 서버"));
    }

    fn tempdir_path(label: &str) -> std::path::PathBuf {
        // 테스트가 병렬 실행되므로 같은 폴더를 두 테스트가 공유하지 않게 label 포함.
        let p = std::env::temp_dir()
            .join(format!("pengport-test-{}-{}", std::process::id(), label));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }
}

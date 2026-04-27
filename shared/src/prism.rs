//! PSP `third_party.prism-launcher` 의 인스턴스 동기화.
//!
//! Service manifest 의 `actions[].args.config` (PrismLauncherConfig) 만 받아
//! Prism Launcher 인스턴스 폴더를 갱신한다 (instance.cfg / mmc-pack.json /
//! servers.dat / packwiz-installer-bootstrap.jar).
//!
//! 정리 (`[보관]`) 정책: Phase 1 단순화 — 한 번 만든 인스턴스는 그대로 둠.
//! 사용자 세이브 보존 우선. 다중 service 가 같은 prism 카탈로그를 공유하는 시나리오에서
//! 보존 처리로 인한 데이터 손실 방지.

use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::actions::third_party::prism_launcher::{PrismLauncherConfig, PrismLoader};
use crate::servers_dat;

#[derive(Debug, Error)]
pub enum PrismError {
    #[error("I/O 실패: {0}")]
    Io(#[from] std::io::Error),

    #[error("Prism 인스턴스 루트를 찾을 수 없습니다: {0}")]
    InstancesNotFound(PathBuf),

    #[error("servers.dat 갱신 실패: {0}")]
    ServersDat(#[from] servers_dat::ServersDatError),
}

/// Prism 설치 위치를 추상화. `instances` 폴더만 필요.
#[derive(Debug, Clone)]
pub struct PrismPaths {
    pub instances: PathBuf,
}

impl PrismPaths {
    pub fn new(instances: PathBuf) -> Self {
        Self { instances }
    }

    pub fn instance_dir(&self, id: &str) -> PathBuf {
        self.instances.join(id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceOutcome {
    Unchanged,
    Updated,
}

/// PSP `third_party.prism-launcher` 의 단일 인스턴스 upsert.
///
/// `instance_id` 는 Prism 데이터 폴더명 (= Prism UI 에 보이는 ID). 보통 service id.
/// `display_name` 은 instance.cfg 의 `name=` 필드. None 이면 `instance_id`.
pub fn upsert_prism_instance(
    paths: &PrismPaths,
    instance_id: &str,
    config: &PrismLauncherConfig,
    bootstrap_jar: &Path,
) -> Result<InstanceOutcome, PrismError> {
    let display = config
        .display_name
        .as_deref()
        .unwrap_or(instance_id)
        .to_string();
    let dir = paths.instance_dir(instance_id);
    let mc_dir = dir.join(".minecraft");
    fs::create_dir_all(&mc_dir)?;

    // packwiz_url 이 없으면 PreLaunchCommand 없는 instance.cfg (vanilla 단순 실행).
    let cfg = match config.packwiz_url.as_deref() {
        Some(url) => render_instance_cfg(&display, url),
        None => render_instance_cfg_no_packwiz(&display),
    };
    let cfg_path = dir.join("instance.cfg");
    let cfg_changed = write_if_changed(&cfg_path, cfg.as_bytes())?;

    let pack = render_mmc_pack_for_prism_loader(
        &config.version,
        config.loader,
        config.loader_version.as_deref(),
    );
    let pack_path = dir.join("mmc-pack.json");
    let pack_changed = write_if_changed(&pack_path, pack.as_bytes())?;

    // bootstrap jar — packwiz_url 이 있을 때만 필요.
    if config.packwiz_url.is_some() {
        let jar_dst = mc_dir.join("packwiz-installer-bootstrap.jar");
        let needs_copy = match (fs::metadata(&jar_dst), fs::metadata(bootstrap_jar)) {
            (Ok(d), Ok(s)) => d.len() != s.len(),
            _ => true,
        };
        if needs_copy {
            fs::copy(bootstrap_jar, &jar_dst)?;
        }
    }

    // servers.dat — Minecraft 멀티플레이어 서버 목록 자동 등록 (사용자 IP 입력 불필요).
    let servers_dat_path = mc_dir.join("servers.dat");
    servers_dat::upsert_server(&servers_dat_path, &display, &config.host, config.port)?;

    Ok(if cfg_changed || pack_changed {
        InstanceOutcome::Updated
    } else {
        InstanceOutcome::Unchanged
    })
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<bool, PrismError> {
    if path.exists() {
        let existing = fs::read(path)?;
        if existing == bytes {
            return Ok(false);
        }
    }
    fs::write(path, bytes)?;
    Ok(true)
}

fn render_instance_cfg(display_name: &str, packwiz_url: &str) -> String {
    // PreLaunchCommand 는 실행 시 `.minecraft/` 가 CWD 이므로 상대 경로.
    format!(
        "InstanceType=OneSix\n\
         iconKey=default\n\
         name={display_name}\n\
         OverrideCommands=true\n\
         PreLaunchCommand=\"$INST_JAVA\" -jar packwiz-installer-bootstrap.jar {packwiz_url}\n"
    )
}

fn render_instance_cfg_no_packwiz(display_name: &str) -> String {
    format!(
        "InstanceType=OneSix\n\
         iconKey=default\n\
         name={display_name}\n"
    )
}

fn render_mmc_pack_for_prism_loader(
    mc_version: &str,
    loader: PrismLoader,
    loader_version: Option<&str>,
) -> String {
    match loader {
        PrismLoader::Vanilla => format!(
            r#"{{
  "components": [
    {{ "important": true, "uid": "net.minecraft", "version": "{mc}" }}
  ],
  "formatVersion": 1
}}
"#,
            mc = mc_version
        ),
        PrismLoader::Fabric => format!(
            r#"{{
  "components": [
    {{ "important": true, "uid": "net.minecraft", "version": "{mc}" }},
    {{ "uid": "net.fabricmc.intermediary", "version": "{mc}" }},
    {{ "uid": "net.fabricmc.fabric-loader", "version": "{lv}" }}
  ],
  "formatVersion": 1
}}
"#,
            mc = mc_version,
            lv = loader_version.unwrap_or("")
        ),
        PrismLoader::Forge => format!(
            r#"{{
  "components": [
    {{ "important": true, "uid": "net.minecraft", "version": "{mc}" }},
    {{ "uid": "net.minecraftforge", "version": "{lv}" }}
  ],
  "formatVersion": 1
}}
"#,
            mc = mc_version,
            lv = loader_version.unwrap_or("")
        ),
        PrismLoader::Neoforge => format!(
            r#"{{
  "components": [
    {{ "important": true, "uid": "net.minecraft", "version": "{mc}" }},
    {{ "uid": "net.neoforged", "version": "{lv}" }}
  ],
  "formatVersion": 1
}}
"#,
            mc = mc_version,
            lv = loader_version.unwrap_or("")
        ),
        PrismLoader::Quilt => format!(
            r#"{{
  "components": [
    {{ "important": true, "uid": "net.minecraft", "version": "{mc}" }},
    {{ "uid": "org.quiltmc.quilt-loader", "version": "{lv}" }}
  ],
  "formatVersion": 1
}}
"#,
            mc = mc_version,
            lv = loader_version.unwrap_or("")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("pengport-prism-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn sample_prism_config(loader: PrismLoader, mc: &str, lv: Option<&str>) -> PrismLauncherConfig {
        PrismLauncherConfig {
            host: "play.example.com".into(),
            port: 25565,
            version: mc.into(),
            loader,
            loader_version: lv.map(String::from),
            packwiz_url: Some("https://cdn.example.com/pack.toml".into()),
            java_major: None,
            display_name: Some("Example".into()),
        }
    }

    #[test]
    fn upsert_prism_instance_creates_files() {
        let instances_dir = temp_dir("upsert-prism-create");
        let paths = PrismPaths::new(instances_dir.clone());
        let jar = temp_dir("upsert-prism-jar").join("bootstrap.jar");
        fs::write(&jar, b"dummy").unwrap();

        let cfg = sample_prism_config(PrismLoader::Fabric, "1.21.1", Some("0.18.4"));
        let outcome = upsert_prism_instance(&paths, "modded", &cfg, &jar).unwrap();
        assert_eq!(outcome, InstanceOutcome::Updated);
        assert!(instances_dir.join("modded/instance.cfg").exists());
        assert!(instances_dir.join("modded/mmc-pack.json").exists());
        assert!(instances_dir
            .join("modded/.minecraft/packwiz-installer-bootstrap.jar")
            .exists());
        assert!(instances_dir.join("modded/.minecraft/servers.dat").exists());

        let outcome2 = upsert_prism_instance(&paths, "modded", &cfg, &jar).unwrap();
        assert_eq!(outcome2, InstanceOutcome::Unchanged);
    }

    #[test]
    fn upsert_prism_instance_vanilla_no_packwiz() {
        let instances_dir = temp_dir("upsert-prism-vanilla");
        let paths = PrismPaths::new(instances_dir.clone());
        let jar = temp_dir("upsert-prism-vjar").join("bootstrap.jar");
        fs::write(&jar, b"dummy").unwrap();

        let mut cfg = sample_prism_config(PrismLoader::Vanilla, "1.21.1", None);
        cfg.packwiz_url = None;

        let outcome = upsert_prism_instance(&paths, "vanilla-svr", &cfg, &jar).unwrap();
        assert_eq!(outcome, InstanceOutcome::Updated);

        let cfg_text =
            fs::read_to_string(instances_dir.join("vanilla-svr/instance.cfg")).unwrap();
        assert!(!cfg_text.contains("PreLaunchCommand"));
        assert!(!cfg_text.contains("packwiz"));
        assert!(!instances_dir
            .join("vanilla-svr/.minecraft/packwiz-installer-bootstrap.jar")
            .exists());

        let pack = fs::read_to_string(instances_dir.join("vanilla-svr/mmc-pack.json")).unwrap();
        assert!(pack.contains("net.minecraft"));
        assert!(!pack.contains("fabric"));
    }

    #[test]
    fn upsert_prism_instance_updates_on_change() {
        let instances_dir = temp_dir("upsert-prism-update");
        let paths = PrismPaths::new(instances_dir.clone());
        let jar = temp_dir("upsert-prism-jar3").join("bootstrap.jar");
        fs::write(&jar, b"dummy").unwrap();

        let cfg1 = sample_prism_config(PrismLoader::Fabric, "1.21.1", Some("0.18.4"));
        upsert_prism_instance(&paths, "modded", &cfg1, &jar).unwrap();

        let cfg2 = sample_prism_config(PrismLoader::Fabric, "1.21.4", Some("0.18.4"));
        let outcome = upsert_prism_instance(&paths, "modded", &cfg2, &jar).unwrap();
        assert_eq!(outcome, InstanceOutcome::Updated);

        let pack = fs::read_to_string(instances_dir.join("modded/mmc-pack.json")).unwrap();
        assert!(pack.contains("1.21.4"));
        assert!(!pack.contains("1.21.1"));
    }

    #[test]
    fn render_prism_pack_vanilla() {
        let out = render_mmc_pack_for_prism_loader("1.21.1", PrismLoader::Vanilla, None);
        assert!(out.contains("net.minecraft"));
        assert!(!out.contains("fabric"));
    }

    #[test]
    fn render_prism_pack_neoforge() {
        let out =
            render_mmc_pack_for_prism_loader("1.21.1", PrismLoader::Neoforge, Some("21.0.42"));
        assert!(out.contains("net.neoforged"));
        assert!(out.contains("21.0.42"));
    }
}

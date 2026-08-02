//! [`LaunchAction`] 검증 — 실행 경로 안전성 + third-party app id 형식.

use super::relative_path::validate_relative_path;
use super::ActionError;
use crate::ids::validate_service_id;
use crate::library::LaunchAction;

pub fn validate_launch_action(action: &LaunchAction) -> Result<(), ActionError> {
    match action {
        LaunchAction::SpawnProcess { entry_point, .. } => {
            if entry_point.trim().is_empty() {
                return Err(ActionError::InvalidConfig(
                    "spawn_process: entry_point 가 비어있음".into(),
                ));
            }
            validate_relative_path(entry_point).map_err(|e| {
                ActionError::InvalidConfig(format!("spawn_process: entry_point 오류: {e}"))
            })
        }
        LaunchAction::ThirdPartyAppLaunch { app_id } => validate_service_id(app_id)
            .map_err(|e| {
                ActionError::InvalidConfig(format!("third_party_app_launch: app_id 오류: {e}"))
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_spawn_process() {
        let action = LaunchAction::SpawnProcess {
            entry_point: "SampleApp/Launcher.exe".to_string(),
            entry_args: vec![],
        };
        assert!(validate_launch_action(&action).is_ok());
    }

    #[test]
    fn rejects_spawn_process_empty_entry_point() {
        let action = LaunchAction::SpawnProcess {
            entry_point: "".to_string(),
            entry_args: vec![],
        };
        let err = validate_launch_action(&action).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn rejects_spawn_process_absolute_entry_point() {
        let action = LaunchAction::SpawnProcess {
            entry_point: "C:/Windows/System32/cmd.exe".to_string(),
            entry_args: vec![],
        };
        let err = validate_launch_action(&action).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn validates_third_party_app_launch() {
        let action = LaunchAction::ThirdPartyAppLaunch {
            app_id: "test_app".to_string(),
        };
        assert!(validate_launch_action(&action).is_ok());
    }

    #[test]
    fn rejects_third_party_app_launch_bad_app_id() {
        let action = LaunchAction::ThirdPartyAppLaunch {
            app_id: "../evil".to_string(),
        };
        let err = validate_launch_action(&action).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }
}

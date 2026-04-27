//! PSP — PengPort Service Protocol 클라이언트 모듈.
//!
//! 명세: `docs/spec/psp-v1.md`
//! 모델: `docs/spec/05-psp.md`
//!
//! 이 모듈은 PSP v1 의 schema 타입과 (이후 라운드에서) manifest 일관성 검증,
//! status / events 클라이언트, catalog 파서를 제공한다.
//!
//! 1 단계 (현재): schema 타입만. 검증 algorithm + HTTP 클라이언트는 후속 라운드.

pub mod catalog;
pub mod events;
pub mod fetch;
pub mod instance;
pub mod manifest;
pub mod status;

pub use fetch::{fetch_instance_metadata, fetch_service_manifest, FetchError};

pub use catalog::{InstanceInfo, ServiceEntry, ServiceHint, ServicesCatalog};
pub use events::{
    CustomEvent, InstanceEvent, NotificationEvent, NotificationLevel, ServiceEvent,
};
pub use instance::{
    InstanceAuth, InstanceAuthType, InstanceEndpoints, InstanceMetadata, OAuth2Endpoints,
    OperatorInfo,
};
pub use manifest::{
    CategoryHint, EventType, ManifestEndpoints, NativeActionKind, Permissions, ServiceAction,
    ServiceManifest,
};
pub use status::{Badge, BadgeLevel, Metric, MetricType, StatusResponse};

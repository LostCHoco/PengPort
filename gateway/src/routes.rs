//! HTTP handlers.
//!
//! - `GET /.well-known/pengport-instance` → InstanceMetadata
//! - `GET /services` → ServicesCatalog (`SERVICES_DIR` merge)
//! - `GET /health` → 200 OK
//!
//! Phase 2 추가: `GET /events` (SSE multiplexer).

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use pengport_shared::psp::catalog::{merge_catalog_dir, ServicesCatalog};
use pengport_shared::psp::instance::{
    InstanceAuth, InstanceAuthType, InstanceEndpoints, InstanceMetadata, OperatorInfo,
};
use serde::Deserialize;
use subtle::ConstantTimeEq;

use crate::config::{AppConfig, SecretString};

#[derive(Clone)]
pub struct AppCtx {
    pub cfg: Arc<AppConfig>,
}

impl AppCtx {
    pub fn new(cfg: AppConfig) -> anyhow::Result<Self> {
        Ok(Self { cfg: Arc::new(cfg) })
    }
}

#[derive(Deserialize)]
pub struct TokenQuery {
    token: Option<String>,
}

fn ct_token_eq(provided: &[u8], expected: &[u8]) -> bool {
    if provided.len() != expected.len() {
        let _ = expected.ct_eq(expected);
        return false;
    }
    provided.ct_eq(expected).into()
}

fn check_token(provided: Option<&str>, expected: Option<&SecretString>) -> Result<(), StatusCode> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let provided = provided.unwrap_or("").as_bytes();
    if ct_token_eq(provided, expected.expose().as_bytes()) {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

pub async fn instance_handler(
    State(ctx): State<AppCtx>,
) -> Result<Json<InstanceMetadata>, StatusCode> {
    let auth_type = match ctx.cfg.instance_auth_type.as_str() {
        "token" => InstanceAuthType::Token,
        "oauth2" => InstanceAuthType::Oauth2,
        _ => InstanceAuthType::None,
    };

    let base = ctx
        .cfg
        .instance_public_base_url
        .clone()
        .unwrap_or_else(|| format!("http://{}", ctx.cfg.bind));
    let base = base.trim_end_matches('/').to_string();

    let metadata = InstanceMetadata {
        schema_version: 1,
        name: ctx.cfg.instance_name.clone(),
        description: ctx.cfg.instance_description.clone(),
        operator: OperatorInfo {
            name: ctx.cfg.operator_name.clone(),
            contact: ctx.cfg.operator_contact.clone(),
        },
        endpoints: InstanceEndpoints {
            catalog: format!("{}/services", base),
            events: None, // Phase 2 multiplexer 도입 시 채움
        },
        auth: InstanceAuth {
            kind: auth_type,
            token_hint: ctx.cfg.instance_token_hint.clone(),
            oauth2: None,
        },
        icon_url: ctx.cfg.instance_icon_url.clone(),
        pengport_min_version: None,
        public_key_fingerprint: None,
    };

    Ok(Json(metadata))
}

pub async fn catalog_handler(
    State(ctx): State<AppCtx>,
    Query(q): Query<TokenQuery>,
) -> Result<Json<ServicesCatalog>, StatusCode> {
    check_token(q.token.as_deref(), ctx.cfg.events_token.as_ref())?;

    if !ctx.cfg.services_dir.is_dir() {
        tracing::warn!(
            dir = %ctx.cfg.services_dir.display(),
            "SERVICES_DIR 가 디렉토리 아님 — 빈 catalog 응답"
        );
        return Ok(Json(ServicesCatalog {
            schema_version: "2".to_string(),
            instance: None,
            services: Vec::new(),
        }));
    }

    let catalog = merge_catalog_dir(&ctx.cfg.services_dir).map_err(|e| {
        tracing::error!(error=%e, "catalog merge 실패");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(catalog))
}

pub async fn health_handler() -> &'static str {
    "ok"
}

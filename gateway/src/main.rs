//! PengPort instance gateway.
//!
//! 외부 클라이언트와 내부 service 어댑터들 사이의 진입점 (검문소).
//!
//! Phase 1 책임: instance metadata + services catalog 정적 서빙.
//! Phase 2 추가 책임: SSE multiplexer (각 service 의 PSP events 를 단일 SSE 로 fan-out).
//!
//! ```text
//! 외부 → GET /.well-known/pengport-instance  → InstanceMetadata
//!     → GET /services                        → ServicesCatalog (services.d/ merge)
//!     [Phase 2] GET /events                  → InstanceEvent SSE
//! ```
//!
//! 카테고리 종속 코드는 별도 어댑터로 이전됨 (트랙 08, `adapter-minecraft/`).
//! gateway 자체는 generic — 어떤 카테고리의 service 든 catalog 에 등록 가능.

mod config;
mod routes;

use anyhow::Result;
use axum::{routing::get, Router};
use tokio::net::TcpListener;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::config::AppConfig;
use crate::routes::AppCtx;

fn init_tracing() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let cfg = AppConfig::from_env()?;
    tracing::info!(
        instance = %cfg.instance_name,
        bind = %cfg.bind,
        services_dir = %cfg.services_dir.display(),
        "gateway 시작"
    );

    let ctx = AppCtx::new(cfg)?;

    let app = Router::new()
        .route("/.well-known/pengport-instance", get(routes::instance_handler))
        .route("/services", get(routes::catalog_handler))
        .route("/health", get(routes::health_handler))
        .with_state(ctx.clone());

    let listener = TcpListener::bind(&ctx.cfg.bind).await?;
    tracing::info!("HTTP listen: http://{}", ctx.cfg.bind);

    // Graceful shutdown — Ctrl+C / SIGTERM 받아서 정상 종료.
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("gateway 종료");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Ctrl+C handler 설치 실패");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler 설치 실패")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("종료 신호 수신");
}

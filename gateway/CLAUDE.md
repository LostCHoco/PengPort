# pengport-gateway

PengPort instance 의 gateway — 외부 클라이언트와 내부 service 어댑터들 사이의 진입점. instance metadata + services catalog 응답 + (Phase 2) SSE multiplexer.

설계: [`docs/spec/05-psp.md`](../docs/spec/05-psp.md) 섹션 8. 보관: [`docs/design/03-event-broadcaster.md`](../docs/design/03-event-broadcaster.md) (옛 broadcaster 시절).

## 파일 구조

```
src/
├── main.rs           ← axum 부트 + graceful shutdown (Ctrl+C / SIGTERM)
├── config.rs         ← 환경변수 (INSTANCE_NAME, INSTANCE_OPERATOR, SERVICES_DIR, EVENTS_TOKEN, ...)
└── routes.rs         ← /.well-known/pengport-instance, /services, /health
Cargo.toml
Dockerfile           ← 멀티스테이지 (rust → debian-slim)
```

## 엔드포인트

| 경로 | 메서드 | 응답 |
|---|---|---|
| `/.well-known/pengport-instance` | GET | `InstanceMetadata` JSON |
| `/services` | GET | `ServicesCatalog` JSON (`SERVICES_DIR` merge 결과). `?token=` 쿼리로 `EVENTS_TOKEN` 검증 |
| `/health` | GET | `"ok"` |

(Phase 2) SSE multiplexer:
| `/events` | GET (SSE) | `InstanceEvent` 스트림 (각 service events 통합) |

## 환경변수

| 변수 | 의미 | 기본값 |
|---|---|---|
| `BIND` | HTTP 리슨 | `0.0.0.0:8080` |
| `INSTANCE_NAME` | InstanceMetadata.name | (필수) |
| `INSTANCE_OPERATOR` | OperatorInfo.name | (필수) |
| `INSTANCE_DESCRIPTION` | InstanceMetadata.description | optional |
| `INSTANCE_OPERATOR_CONTACT` | OperatorInfo.contact | optional |
| `INSTANCE_ICON_URL` | InstanceMetadata.icon_url | optional |
| `INSTANCE_AUTH_TYPE` | `none` / `token` / `oauth2` | `none` |
| `INSTANCE_TOKEN_HINT` | `auth.type=token` 시 안내 | optional |
| `INSTANCE_PUBLIC_BASE_URL` | metadata 의 endpoints URL prefix | `http://{BIND}` |
| `SERVICES_DIR` | services.d/ 디렉토리 경로 | `./services.d` |
| `EVENTS_TOKEN` | catalog 보호 토큰 (constant-time 비교) | optional |

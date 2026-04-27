# PengPort Platform

Minecraft 서버 플랫폼 (Tauri + React) + Event Broadcaster (Rust axum SSE).

## 구조

```
.
├── Cargo.toml                  # workspace root
├── servers.toml                # SSOT — 서버 목록/메타
├── shared/                     # Rust 공통 크레이트
├── player-launcher/            # Tauri 2 + React 19 앱
│   ├── src/                    # React 프론트
│   └── src-tauri/              # Rust 백엔드
├── event-broadcaster/          # Oracle 배포용 SSE 서비스
├── servers/event-broadcaster/  # Docker compose + Caddy
├── scripts/                    # 빌드 스크립트
└── docs/design/                # 설계 문서
```

## 개발

### 플랫폼 앱 (dev 모드)

```bash
cd player-launcher
npm install
npm run tauri dev
```

로컬 개발 시 Prism 경로를 환경변수로 override:

```bash
PENGPORT_PRISM_ROOT="C:/path/to/PrismLauncher" npm run tauri dev
```

### 테스트

```bash
cargo test -p pengport-shared
```

### Event Broadcaster (로컬)

```bash
cargo run -p event-broadcaster
```

환경변수 필요 — `servers/event-broadcaster/.env.example` 참고.

## 배포

### 플랫폼 앱 빌드

```bash
python scripts/build_client.py
# 결과: client/build/펭돌서버-플랫폼-v3.zip
```

### Event Broadcaster (Oracle)

```bash
# Oracle 서버에서
cd ~/pengport-workspace/servers/event-broadcaster
docker compose up -d --build
```

## 진행 중 작업

진행 항목은 [`docs/track/`](docs/track/) 에서 트랙 단위로 관리.

### PSP 비전 트랙 (2026-04-27 신설)
- [07-psp-spec](docs/track/07-psp-spec.md) — PSP v1 명세 작성 (모든 후속 트랙의 기반)
- [08-mc-adapter-extraction](docs/track/08-mc-adapter-extraction.md) — Minecraft 어댑터 별도 저장소 분리
- [09-broadcaster-simplification](docs/track/09-broadcaster-simplification.md) — broadcaster 를 단순 SSE multiplexer 로
- [10-native-actions-v1](docs/track/10-native-actions-v1.md) — 표준 native action handlers (open_url, native_minecraft_play 등)
- [11-instance-metadata](docs/track/11-instance-metadata.md) — `/.well-known/pengport-instance` endpoint
- [12-services-d-pattern](docs/track/12-services-d-pattern.md) — `services.d/` 디렉토리 catalog 패턴
- [13-pengport-discovery](docs/track/13-pengport-discovery.md) — Phase 2 Docker labels 자동 발견 (별도 저장소)
- [14-psp-security-model](docs/track/14-psp-security-model.md) — 3-tier 신뢰 + permissions 검증 + Tier 2/3 UI
- [15-supply-chain](docs/track/15-supply-chain.md) — cargo/npm audit, Dependabot
- [16-token-rotation](docs/track/16-token-rotation.md) — 토큰 회전 자동화 인프라 (Phase 2 OAuth 토대, P2)

### 기존 트랙
- [00-security-hardening](docs/track/00-security-hardening.md) — 토큰·CSP·시크릿 관리 (events_token 처리는 트랙 11/14 와 결합)
- [01-runtime-stability](docs/track/01-runtime-stability.md) — panic·race·무시되는 에러
- [02-docs-migration](docs/track/02-docs-migration.md) — design 보관 + spec/ 신설
- [03-ci-automation](docs/track/03-ci-automation.md) — GitHub Actions, healthcheck, engines, audit
- [04-test-coverage](docs/track/04-test-coverage.md) — 단위/통합 테스트 보강
- [05-code-quality](docs/track/05-code-quality.md) — 잔재·하드코딩·타입 동기화
- [06-readme-followups](docs/track/06-readme-followups.md) — 관리자 앱·packwiz merge

## 문서

### 현행 명세 (`docs/spec/`)
- [04-vision.md](docs/spec/04-vision.md) — PengPort 비전 + 4대 핵심 원칙 + 결정 사항
- [05-psp.md](docs/spec/05-psp.md) — PSP (PengPort Service Protocol) 명세 + 3-tier 보안 모델

### 보관 (`docs/design/`)
- [01-launcher-architecture.md](docs/design/01-launcher-architecture.md) — 초기 단순 런처 (보관)
- [02-player-platform.md](docs/design/02-player-platform.md) — 스팀-like 플랫폼 초안 (보관, 04/05 로 대체)
- [03-event-broadcaster.md](docs/design/03-event-broadcaster.md) — SSE 브로드캐스터 초안 (보관, 05 섹션 8 로 대체)

## 라이선스

[AGPL-3.0-only](LICENSE).

PengPort 는 자유·무료 소프트웨어다. 누구나 다운로드·사용·수정·재배포할 수 있으며, 운영자는 자유롭게 자신의 인스턴스를 띄울 수 있다. 단 수정 사항을 재배포하거나 네트워크로 서비스 제공하는 경우 같은 라이선스로 소스 공개 의무가 있다 (AGPL 의 network clause).

비영리 프로젝트 — 과금 모델은 두지 않으며, 핵심 개발 운영비는 후원으로만 충당한다. 운영자가 자신의 PengPort 인스턴스 위에서 별도 비즈니스를 만드는 것은 자유다.

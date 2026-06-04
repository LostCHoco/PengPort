# PengPort

자체 호스팅 서비스 통합 데스크탑 클라이언트. 친구 그룹용 인스턴스의 서비스 카탈로그(서버, 미디어, 파일 등)를 한 곳에서 다룬다.

## 이 repo

이 repo 는 **클라이언트 (Tauri + React) + PSP schema 라이브러리** 만 포함합니다. PSP 정신상 server-side 컴포넌트는 별도 repo:

| repo | 책임 |
|---|---|
| **이 repo** ([LostCHoco/PengPort](https://github.com/LostCHoco/PengPort)) | client (player-launcher) + shared (PSP schema lib) |
| [LostCHoco/pengport-gateway](https://github.com/LostCHoco/pengport-gateway) | instance gateway — instance metadata + catalog 서빙 |
| [LostCHoco/pengport-adapter-minecraft](https://github.com/LostCHoco/pengport-adapter-minecraft) | MC 카테고리 어댑터 |
| (다른 카테고리 어댑터 — 각자 별도 repo) | jellyfin, nextcloud, ... |

## 구조

```
.
├── Cargo.toml                  # workspace root
├── shared/                     # PSP schema + actions + trust + prism + servers_dat
├── player-launcher/            # Tauri 2 + React 19 앱
│   ├── src/                    # React 프론트
│   └── src-tauri/              # Rust 백엔드
├── servers/gateway/            # 펭돌서버 운영 docker-compose + Caddyfile + services.d/
│                               # (다음 라운드: pengdoll-ops private repo 로 이전 예정)
├── scripts/                    # 빌드 + 운영 마이그레이션 스크립트
└── docs/                       # spec/, design/(보관), guide/, note/, track/
```

## 개발

### 클라이언트 (dev 모드)

```bash
cd player-launcher
pnpm install
pnpm tauri dev
```

로컬 개발 시 Prism 경로를 환경변수로 override:

```bash
PENGPORT_PRISM_ROOT="C:/path/to/PrismLauncher" pnpm tauri dev
```

### 테스트

```bash
cargo test --workspace
```

### Gateway / Adapter 로컬

각각 별도 repo 에서:

```bash
git clone https://github.com/LostCHoco/pengport-gateway && cd pengport-gateway && cargo run
git clone https://github.com/LostCHoco/pengport-adapter-minecraft && cd pengport-adapter-minecraft && cargo run
```

환경변수 필요 — 각 repo 의 README 참고.

## 배포

### 클라이언트 빌드

```bash
python scripts/build_client.py
# 결과: client/build/PengPort-v*.zip
```

### Gateway 인스턴스 (운영자용)

운영 마이그레이션 가이드: [`docs/guide/psp-migration.md`](docs/guide/psp-migration.md).
펭돌서버 운영 환경: 사용자 사적 (oracle 의 `~/pengdoll-ops/`).

## 진행 중 작업

진행 항목은 [`docs/track/`](docs/track/) 에서 트랙 단위로 관리.

### PSP 비전 트랙 (2026-04-27 완료)
- [07-psp-spec](docs/track/07-psp-spec.md) — PSP v1 명세 ✅
- [08-mc-adapter-extraction](docs/track/08-mc-adapter-extraction.md) — Minecraft 어댑터 별도 repo ✅
- [09-broadcaster-simplification](docs/track/09-broadcaster-simplification.md) — gateway (옛 broadcaster) 단순화 ✅
- [10-native-actions-v1](docs/track/10-native-actions-v1.md) — 표준 native action handlers ✅
- [11-instance-metadata](docs/track/11-instance-metadata.md) — `/.well-known/pengport-instance` endpoint ✅
- [12-services-d-pattern](docs/track/12-services-d-pattern.md) — `services.d/` 디렉토리 catalog ✅
- [14-psp-security-model](docs/track/14-psp-security-model.md) — 3-tier 신뢰 + Tier 1/2/3 UI ✅
- [13-pengport-discovery](docs/track/13-pengport-discovery.md) — Phase 2 보류
- [15-supply-chain](docs/track/15-supply-chain.md) — 미시작
- [16-token-rotation](docs/track/16-token-rotation.md) — 미시작

### 품질 트랙 (잔여)
- [00-security-hardening](docs/track/00-security-hardening.md) — P0 3건 완료
- [01-runtime-stability](docs/track/01-runtime-stability.md)
- [02-docs-migration](docs/track/02-docs-migration.md) — 완료
- [03-ci-automation](docs/track/03-ci-automation.md) — 미시작
- [04-test-coverage](docs/track/04-test-coverage.md)
- [05-code-quality](docs/track/05-code-quality.md)
- [06-readme-followups](docs/track/06-readme-followups.md)

## 문서

### 현행 명세 (`docs/spec/`)
- [04-vision.md](docs/spec/04-vision.md) — PengPort 비전 + 4대 핵심 원칙
- [05-psp.md](docs/spec/05-psp.md) — PSP (PengPort Service Protocol) 모델 + 3-tier 보안
- [psp-v1.md](docs/spec/psp-v1.md) — PSP v1 정밀 명세 (RFC 2119)

### 가이드 (`docs/guide/`)
- [psp-migration.md](docs/guide/psp-migration.md) — 운영 인스턴스 PSP 마이그레이션
- [adapter-separation.md](docs/guide/adapter-separation.md) — 카테고리 어댑터 별도 repo 분리

### 보관 (`docs/design/`)
- [01-launcher-architecture.md](docs/design/01-launcher-architecture.md) — 초기 단순 런처 (보관)
- [02-player-platform.md](docs/design/02-player-platform.md) — 스팀-like 플랫폼 초안 (보관, 04/05 로 대체)
- [03-event-broadcaster.md](docs/design/03-event-broadcaster.md) — 옛 broadcaster 초안 (보관, 05 섹션 8 의 gateway 로 대체)

## 라이선스

[AGPL-3.0-only](LICENSE).

PengPort 는 자유·무료 소프트웨어다. 누구나 다운로드·사용·수정·재배포할 수 있으며, 운영자는 자유롭게 자신의 인스턴스를 띄울 수 있다. 단 수정 사항을 재배포하거나 네트워크로 서비스 제공하는 경우 같은 라이선스로 소스 공개 의무가 있다 (AGPL 의 network clause).

비영리 프로젝트 — 과금 모델은 두지 않으며, 핵심 개발 운영비는 후원으로만 충당한다. 운영자가 자신의 PengPort 인스턴스 위에서 별도 비즈니스를 만드는 것은 자유다.

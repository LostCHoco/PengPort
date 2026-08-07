# PengPort

앱(자기 소유 self-hosted 서비스든 상용 제3자 앱이든 무관)의 설치→인증→실행을 원클릭으로 자동화하고, 그렇게 관리되는 앱들을 flat한 라이브러리로 한눈에 보여주는 데스크탑 클라이언트.

카탈로그·인스턴스 개념은 없다 — 라이브러리(로컬 레시피 목록)가 유일한 데이터 구조다. 항목 추가는 직접 등록하거나, `.pengz` 파일(스냅샷 번들)을 열어 임포트한다. 신뢰는 두 층: ①설치 아티팩트는 항상 서명/해시로 자동 검증 ②`.pengz` 임포트 시 포함 항목 전체를 1회만 confirm — 그 이후 실행은 항상 완전 원클릭.

## 다운로드

[Releases](https://github.com/LostCHoco/PengPort/releases)에서 최신 버전의 포터블 zip을 받아 압축을 풀고 `PengPort.exe`를 실행하면 된다. 설치 프로그램 없음 — 원클릭 컨셉상 그대로 실행만 하면 된다.

## 이 repo

이 repo는 **클라이언트 (Tauri + React) + 레시피 schema 라이브러리**만 포함한다. 카탈로그/인스턴스 모델 시절의 서버측 컴포넌트는 더 이상 필요 없어 정리됨:

| repo | 상태 |
|---|---|
| **이 repo** ([LostCHoco/PengPort](https://github.com/LostCHoco/PengPort)) | client(player-launcher) + shared(레시피 schema/검증/trust/prism sync) |
| [LostCHoco/pengport-gateway](https://github.com/LostCHoco/pengport-gateway) | **archived** — 인스턴스 카탈로그 상시 호스팅 전제가 없어짐 |
| [LostCHoco/pengport-adapter-minecraft](https://github.com/LostCHoco/pengport-adapter-minecraft) | **archived** — 최소 상태(설치됨/실행중)는 로컬 프로세스 기준이라 원격 상태 서버가 불필요해짐 |

## 라이선스

[GPL-3.0-only](LICENSE).

PengPort 는 자유·무료 소프트웨어다. 누구나 다운로드·사용·수정·재배포할 수 있다. 단 수정 사항을 재배포하는 경우 같은 라이선스로 소스 공개 의무가 있다.

비영리 프로젝트 — 과금 모델은 두지 않으며, 핵심 개발 운영비는 후원으로만 충당한다.

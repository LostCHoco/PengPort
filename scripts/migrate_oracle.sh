#!/usr/bin/env bash
# PengPort Oracle 인스턴스 — PSP 단방향 마이그레이션 스크립트.
#
# 운영 인스턴스의 워크스페이스에서 SSH 후 실행:
#   ssh pengdoll-oracle
#   cd ~/pengport-workspace
#   git pull
#   ./scripts/migrate_oracle.sh
#
# 이 스크립트가 하는 일:
#   1. .env 파일 존재 / 필수 변수 확인
#   2. 이전 gateway 컨테이너 정상 종료 + 백업 태그
#   3. 새 워크스페이스 빌드 (gateway + adapter-minecraft)
#   4. 새 docker-compose 기동 (gateway + adapter-modded + adapter-rlcraft + caddy)
#   5. health endpoint 응답 대기 → 정상 시 안내, 실패 시 안내 + 자동 롤백 옵션
#
# 롤백: `./scripts/rollback_oracle.sh`

set -euo pipefail

WORKDIR="$(cd "$(dirname "$0")/.." && pwd)"
COMPOSE_DIR="$WORKDIR/servers/gateway"
ENV_FILE="$COMPOSE_DIR/.env"
HEALTH_TIMEOUT_SEC=120
HEALTH_INTERVAL_SEC=3

# ANSI 색상.
red()    { printf "\033[31m%s\033[0m\n" "$1" >&2; }
green()  { printf "\033[32m%s\033[0m\n" "$1"; }
yellow() { printf "\033[33m%s\033[0m\n" "$1"; }
blue()   { printf "\033[36m%s\033[0m\n" "$1"; }

step() {
    blue "=========================================="
    blue " $1"
    blue "=========================================="
}

abort() {
    red "❌ $1"
    exit 1
}

# ---------- 1. pre-flight ----------
step "1/5 · 환경 검증"

cd "$COMPOSE_DIR"

[ -f "$ENV_FILE" ] || abort ".env 파일 없음 ($ENV_FILE). .env.example 복사 후 값 채우세요."

# .env 의 필수 변수 확인.
required=(EVENTS_TOKEN INSTANCE_NAME INSTANCE_OPERATOR INSTANCE_PUBLIC_BASE_URL DUCKDNS_TOKEN RCON_MODDED_PASSWORD RCON_RLCRAFT_PASSWORD)
missing=()
# shellcheck disable=SC1090
set -a; source "$ENV_FILE"; set +a
for v in "${required[@]}"; do
    if [ -z "${!v:-}" ] || [[ "${!v}" == *"please-replace"* ]] || [[ "${!v}" == *"your-"*"-here"* ]]; then
        missing+=("$v")
    fi
done
if [ ${#missing[@]} -gt 0 ]; then
    abort ".env 의 다음 변수가 비었거나 placeholder 입니다: ${missing[*]}"
fi
green "✅ .env 검증 통과"

# Docker 동작 확인.
command -v docker >/dev/null 2>&1 || abort "docker 명령이 없습니다."
docker info >/dev/null 2>&1 || abort "docker daemon 에 연결할 수 없습니다 (sudo 필요?)."
green "✅ Docker 사용 가능"

# 외부 네트워크 (modded_default, rlcraft_default) 확인 — 없으면 MC 컨테이너 미기동 상태.
for net in modded_default rlcraft_default; do
    if ! docker network ls --format '{{.Name}}' | grep -qx "$net"; then
        yellow "⚠️  외부 네트워크 '$net' 없음 — MC 컨테이너가 먼저 기동되어 있어야 합니다."
        yellow "   adapter-$net 가 RCON 접근 불가 시 어댑터는 offline 표시되며 기동은 됩니다."
    fi
done

# ---------- 2. 이전 컨테이너 백업 + 정지 ----------
step "2/5 · 기존 컨테이너 정지 + 백업 태그"

# 이전 gateway 가 살아있는지.
if docker ps --format '{{.Names}}' | grep -qx "gateway"; then
    timestamp="$(date +%Y%m%d-%H%M%S)"
    docker tag pengport/gateway:latest "pengport/gateway:rollback-${timestamp}" 2>/dev/null || true
    yellow "ℹ️  이전 image 를 'pengport/gateway:rollback-${timestamp}' 로 태그"
fi

docker compose down --remove-orphans
green "✅ 이전 컨테이너 정지"

# ---------- 3. 빌드 ----------
step "3/5 · workspace 빌드 (gateway + adapter-minecraft + caddy)"

docker compose build --pull
green "✅ 이미지 빌드 완료"

# ---------- 4. 기동 ----------
step "4/5 · 컨테이너 기동"

docker compose up -d
green "✅ docker compose up 완료"

# ---------- 5. health 대기 ----------
step "5/5 · health endpoint 응답 대기"

deadline=$(( $(date +%s) + HEALTH_TIMEOUT_SEC ))
endpoint="${INSTANCE_PUBLIC_BASE_URL}/health"
last_err=""
while [ "$(date +%s)" -lt "$deadline" ]; do
    if response=$(curl -fsS --max-time 5 "$endpoint" 2>&1); then
        if [ "$response" = "ok" ]; then
            green "✅ gateway /health 응답 OK"
            break
        fi
        last_err="응답 본문: $response"
    else
        last_err="$response"
    fi
    sleep "$HEALTH_INTERVAL_SEC"
done

if [ "$(date +%s)" -ge "$deadline" ]; then
    red "❌ gateway health timeout (${HEALTH_TIMEOUT_SEC}초)"
    red "   마지막 에러: $last_err"
    yellow "   로그 확인: docker compose logs -f gateway caddy"
    yellow "   롤백: ./scripts/rollback_oracle.sh"
    exit 2
fi

# ---------- 마무리 ----------
green ""
green "🎉 마이그레이션 완료"
green ""
yellow "검증 스크립트로 모든 PSP endpoint 응답 확인:"
yellow "  ./scripts/verify_psp.sh"
yellow ""
yellow "로그 모니터:"
yellow "  docker compose logs -f gateway adapter-modded adapter-rlcraft caddy"
yellow ""
yellow "문제 시 롤백:"
yellow "  ./scripts/rollback_oracle.sh"

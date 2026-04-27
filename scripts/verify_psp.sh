#!/usr/bin/env bash
# 마이그레이션 후 PSP endpoint 응답 검증.
#
# Oracle 인스턴스에서 SSH 후 또는 외부에서 실행 가능 (URL 외부 접근만 되면).
# 환경변수:
#   INSTANCE_PUBLIC_BASE_URL — 검증할 instance URL. .env 에서 읽음 (또는 export)
#   EVENTS_TOKEN             — catalog/events 인증 토큰. .env 에서 읽음
#
# 검증 항목:
#   1. /.well-known/pengport-instance       — InstanceMetadata JSON
#   2. /services?token=...                  — ServicesCatalog (services.d/ merge 결과)
#   3. /health                              — "ok"
#   4. 각 service 의 /.well-known/pengport-service  — ServiceManifest
#   5. 각 service 의 /pengport/status               — StatusResponse

set -euo pipefail

WORKDIR="$(cd "$(dirname "$0")/.." && pwd)"
ENV_FILE="$WORKDIR/servers/gateway/.env"

[ -f "$ENV_FILE" ] || { echo "❌ .env 없음: $ENV_FILE"; exit 1; }
# shellcheck disable=SC1090
set -a; source "$ENV_FILE"; set +a

: "${INSTANCE_PUBLIC_BASE_URL:?INSTANCE_PUBLIC_BASE_URL 가 .env 에 없음}"
: "${EVENTS_TOKEN:?EVENTS_TOKEN 가 .env 에 없음}"

red()    { printf "\033[31m%s\033[0m\n" "$1" >&2; }
green()  { printf "\033[32m%s\033[0m\n" "$1"; }
yellow() { printf "\033[33m%s\033[0m\n" "$1"; }
blue()   { printf "\033[36m%s\033[0m\n" "$1"; }

failed=0
check() {
    local label="$1"
    local url="$2"
    local jq_filter="${3:-.}"  # 옵션: 응답이 JSON 이면 jq 로 일부만 확인

    blue "  → $url"
    if ! body=$(curl -fsS --max-time 10 "$url" 2>&1); then
        red "    ❌ $label — HTTP 실패: $body"
        failed=$((failed + 1))
        return
    fi
    if command -v jq >/dev/null 2>&1 && [ "$jq_filter" != "raw" ]; then
        if echo "$body" | jq -e "$jq_filter" >/dev/null 2>&1; then
            green "    ✅ $label"
        else
            red "    ❌ $label — JSON 형식 불일치"
            echo "       응답: $(echo "$body" | head -c 200)"
            failed=$((failed + 1))
        fi
    else
        green "    ✅ $label (raw, jq 없음)"
    fi
}

# ---------- 1. instance metadata ----------
blue ""
blue "1) Instance metadata"
check "instance" \
      "$INSTANCE_PUBLIC_BASE_URL/.well-known/pengport-instance" \
      '.schema_version == 1 and .name and .endpoints.catalog'

# ---------- 2. catalog ----------
blue ""
blue "2) Services catalog"
catalog_url="$INSTANCE_PUBLIC_BASE_URL/services?token=$EVENTS_TOKEN"
check "catalog" "$catalog_url" '.schema_version and (.services | type == "array")'

# 서비스 목록 추출 (jq 있으면).
service_ids=()
service_urls=()
if command -v jq >/dev/null 2>&1; then
    while IFS=$'\t' read -r id url enabled; do
        if [ "$enabled" = "true" ]; then
            service_ids+=("$id")
            service_urls+=("$url")
        fi
    done < <(curl -fsS --max-time 10 "$catalog_url" 2>/dev/null \
             | jq -r '.services[] | "\(.id)\t\(.url)\t\(.enabled)"')
fi

# ---------- 3. health ----------
blue ""
blue "3) Health"
if body=$(curl -fsS --max-time 10 "$INSTANCE_PUBLIC_BASE_URL/health" 2>&1); then
    if [ "$body" = "ok" ]; then
        green "    ✅ /health → ok"
    else
        red "    ❌ /health 응답 비정상: $body"
        failed=$((failed + 1))
    fi
else
    red "    ❌ /health HTTP 실패: $body"
    failed=$((failed + 1))
fi

# ---------- 4-5. service manifests + status ----------
if [ ${#service_ids[@]} -eq 0 ]; then
    yellow ""
    yellow "ℹ️  service 없음 (catalog 비었거나 jq 미설치). manifest/status 검증 skip."
else
    for i in "${!service_ids[@]}"; do
        sid="${service_ids[$i]}"
        surl="${service_urls[$i]}"
        blue ""
        blue "4-$((i+1))) Manifest [$sid]"
        check "manifest [$sid]" \
              "$surl/.well-known/pengport-service" \
              '.schema_version == 1 and .id and (.actions | type == "array") and .permissions'

        blue ""
        blue "5-$((i+1))) Status [$sid]"
        check "status [$sid]" \
              "$surl/pengport/status" \
              '.online != null'
    done
fi

# ---------- 결과 ----------
echo ""
if [ "$failed" -eq 0 ]; then
    green "✅ 모든 검증 통과"
    exit 0
else
    red "❌ 실패 항목: $failed"
    yellow "   로그 확인: cd servers/gateway && docker compose logs --tail 50 gateway adapter-modded adapter-rlcraft caddy"
    exit 1
fi

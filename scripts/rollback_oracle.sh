#!/usr/bin/env bash
# 마이그레이션 롤백.
#
# 옵션 1 (코드 + 컨테이너 모두): git 의 이전 커밋으로 되돌리고 docker compose 재기동.
# 옵션 2 (image 만): migrate_oracle.sh 가 태그한 'rollback-<timestamp>' image 로 복귀.
#
# 사용법:
#   ./scripts/rollback_oracle.sh                      # 옵션 1 — 인터랙티브 (git log 표시 → commit 선택)
#   ./scripts/rollback_oracle.sh --image-only         # 옵션 2
#   ./scripts/rollback_oracle.sh --commit <SHA>       # 옵션 1 — 특정 commit

set -euo pipefail

WORKDIR="$(cd "$(dirname "$0")/.." && pwd)"
COMPOSE_DIR="$WORKDIR/servers/gateway"

red()    { printf "\033[31m%s\033[0m\n" "$1" >&2; }
green()  { printf "\033[32m%s\033[0m\n" "$1"; }
yellow() { printf "\033[33m%s\033[0m\n" "$1"; }
blue()   { printf "\033[36m%s\033[0m\n" "$1"; }

abort() { red "❌ $1"; exit 1; }

mode="git"
target_commit=""
while [ $# -gt 0 ]; do
    case "$1" in
        --image-only) mode="image"; shift ;;
        --commit) target_commit="${2:-}"; shift 2 ;;
        --help|-h)
            grep '^#' "$0" | sed 's/^# \?//'
            exit 0
            ;;
        *) abort "알 수 없는 인자: $1" ;;
    esac
done

cd "$COMPOSE_DIR"

# ---------- 옵션 2: image-only 롤백 ----------
if [ "$mode" = "image" ]; then
    blue "image-only 롤백"
    backups=$(docker images --format '{{.Repository}}:{{.Tag}}' \
              | grep '^pengport/gateway:rollback-' || true)
    if [ -z "$backups" ]; then
        abort "rollback image 없음 (마이그레이션 시 자동 태그됐어야)."
    fi
    blue "사용 가능한 rollback image:"
    echo "$backups" | nl
    echo ""
    read -rp "복귀할 번호: " idx
    pick=$(echo "$backups" | sed -n "${idx}p")
    [ -n "$pick" ] || abort "잘못된 선택"
    yellow "→ $pick 으로 latest 재태그"
    docker tag "$pick" pengport/gateway:latest
    docker compose down
    docker compose up -d --no-build
    green "✅ image 롤백 완료. /health 확인 후 정상 동작 검증하세요."
    exit 0
fi

# ---------- 옵션 1: git 커밋 롤백 ----------
cd "$WORKDIR"
[ -d .git ] || abort "git repo 가 아님 ($WORKDIR)"

if [ -z "$target_commit" ]; then
    blue "최근 commit (적절한 commit 선택):"
    git log --oneline -20
    echo ""
    read -rp "복귀할 commit SHA (또는 'q' 취소): " target_commit
    [ "$target_commit" = "q" ] && { yellow "취소"; exit 0; }
fi

[ -n "$target_commit" ] || abort "commit SHA 가 비어 있음"
git rev-parse --verify "$target_commit" >/dev/null 2>&1 || abort "commit '$target_commit' 가 존재하지 않음"

current="$(git rev-parse HEAD)"
yellow "현재 HEAD: $current"
yellow "복귀 대상: $target_commit"
read -rp "이 commit 으로 reset --hard 진행? (y/N) " confirm
[ "$confirm" = "y" ] || [ "$confirm" = "Y" ] || { yellow "취소"; exit 0; }

git reset --hard "$target_commit"
green "✅ git reset 완료"

cd "$COMPOSE_DIR"
docker compose down
docker compose build --pull
docker compose up -d
green "✅ docker compose 재기동 완료"

yellow "검증: ./scripts/verify_psp.sh"
yellow "재롤포워드: git reset --hard $current"

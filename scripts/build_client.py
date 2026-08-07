#!/usr/bin/env python3
"""
build_client.py — PengPort 배포 번들 빌드.

산출물 — 설치 프로그램(NSIS) 없음, 포터블 전용(2026-08 확정: NSIS installer가 자체
업데이트 흐름에서 실행 파일을 설치 마법사로 바꿔치기하는 사고를 냈고, 애초에 "원클릭
실행" 컨셉상 설치 과정 자체가 불필요 — 첫 배포도 포터블 zip 하나로 통일):
1) Portable zip   (`client/build/PengPort-{version}.zip`)
     PengPort.exe                # Tauri release 바이너리 — 원클릭 컨셉상 안내문 없음
2) Release assets (`client/build/release-{version}/`)
     PengPort-{X.Y.Z}.exe            # 위 exe와 같은 바이너리, 버전 붙은 이름 — 자동
                                      # 업데이트(자체 rename-to-delete 업데이터)가 받는 대상
     PengPort-{X.Y.Z}.exe.sig        # 위 exe의 minisign 서명
     latest.json                     # Tauri updater manifest, url → 위 exe
                                      # (GitHub Release, releases/latest/download/...)

PrismLauncher 는 더 이상 번들링하지 않는다. PengPort 가 첫 실행 시 시스템에 설치된
Prism 을 자동 탐색하며, 못 찾으면 OOBE 에서 안내한다 (Phase 3 에서 자동 다운로드 추가 예정).

사용:
    python scripts/build_client.py [--version v3]

전제:
- Rust toolchain + pnpm 설치되어 있어야 함
- `.secrets/pengport-updater.key` 가 있으면 자동 서명 (`.sig` 생성)
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
import zipfile

# Windows console default codec (cp949/cp1252) 가 한국어 print 시 UnicodeEncodeError.
# laptop 빌드 + GH Actions windows-latest 양쪽에서 동일 문제이므로 코드 측에서 utf-8 강제.
if sys.platform == "win32":
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")

ROOT = Path(__file__).resolve().parents[1]
TAURI_CONF = ROOT / "player-launcher" / "src-tauri" / "tauri.conf.json"

# Windows 의 pnpm 은 .cmd 파일이므로 subprocess 가 찾도록 확장자 명시.
PNPM = "pnpm.cmd" if sys.platform == "win32" else "pnpm"

# Self-hosted update endpoint 대신 GitHub Release 사용(2026-08 전환) — latest.json이
# 오라클(공개·인증없는 endpoint)에 있으면 그 자체가 악의적 트래픽 폭주의 표적이 되고,
# Oracle Free Tier는 트래픽 초과 시 차단이 아니라 그대로 과금된다(EDoS 위험). GitHub의
# `releases/latest/download/<파일명>`은 태그가 바뀌어도 항상 최신 release로 연결되는
# 안정 주소라 updater 가 필요로 하는 "고정 폴링 URL" 요건을 그대로 만족한다. GitHub은
# 이런 트래픽에 사실상 무제한이고 과금도 없음. instance 별로 repo가 다르므로 환경변수
# override 가능 (다른 instance fork).
UPDATES_BASE_URL = os.environ.get(
    "PENGPORT_UPDATES_BASE_URL",
    "https://github.com/LostCHoco/PengPort/releases/latest/download",
)


def read_tauri_conf() -> dict:
    return json.loads(TAURI_CONF.read_text(encoding="utf-8"))


def run(cmd: list[str], cwd: Path | None = None) -> None:
    print(f"$ {' '.join(str(c) for c in cmd)}  (cwd={cwd or ROOT})")
    subprocess.run(cmd, cwd=cwd or ROOT, check=True)


def build_tauri() -> Path:
    """Tauri release 빌드 실행. 결과 exe 경로 반환 — bundling은 tauri.conf.json의
    `bundle.active: false`로 항상 꺼져 있어(설치 프로그램 없음, 포터블 전용) 순수
    바이너리 컴파일만 한다. 서명은 이 함수가 아니라 `sign_file`/
    `collect_and_sign_raw_exe`가 별도로 처리(raw exe에 직접)."""
    env = os.environ.copy()
    # sccache 미사용 강제 — release 빌드는 새 의존성이 추가된 직후처럼 미캐시
    # 컴파일이 한꺼번에 몰릴 때 Windows 전용 레이스(sccache 서버가 동시에 여러
    # rustc 를 스폰할 때 발생, 업스트림 수년째 미해결 — mozilla/sccache#1098 등)로
    # "error writing dependencies ... 액세스가 거부되었습니다 (os error 5)" 가
    # 간헐적으로 터짐 (2026-08 확인, 재현 A/B 테스트로 sccache 가 필요조건임을 검증).
    # release 는 배포 시점에만 드물게 돌아 캐시 이득 손실이 미미하므로, `pnpm tauri dev`
    # 개발 루프(캐시 히트가 잦아 sccache 이득이 큼)는 그대로 두고 이 경로만 우회한다.
    env["RUSTC_WRAPPER"] = ""

    run_env([PNPM, "install", "--frozen-lockfile"], cwd=ROOT / "player-launcher", env=env)
    run_env([PNPM, "run", "tauri", "build"], cwd=ROOT / "player-launcher", env=env)
    # Cargo crate 이름이 'pengport' 이라 산출물도 같은 이름.
    exe = ROOT / "target" / "release" / "pengport.exe"
    if not exe.exists():
        raise FileNotFoundError(f"Tauri 빌드 산출물을 찾을 수 없습니다: {exe}")
    return exe


def sign_file(path: Path) -> Path:
    """`path`를 updater 서명 키로 minisign 서명 — `tauri signer sign`(Tauri CLI)을
    그대로 재사용해서 `<path>.sig`를 만든다. 반환값은 그 서명 파일 경로.

    키는 `-f`(파일 경로)가 아니라 `build_tauri()`와 똑같이 **내용을 읽어서 strip한
    뒤 `TAURI_SIGNING_PRIVATE_KEY` 환경변수로 전달**한다 — `-f`는 파일 바이트를
    그대로 읽어서, CI가 시크릿을 `printf '%s\\n'`으로 복원할 때 붙는 trailing
    개행까지 키 내용으로 오인해 "Invalid symbol 10"(개행 문자) 디코드 에러로
    실패했다(2026-08 실측, 로컬에선 파일에 그 개행이 없어서 안 걸렸음). 이 프로젝트
    안에 이미 검증된 경로(env var, strip 됨)를 놔두고 새 경로를 만든 게 원인이라
    같은 방식으로 통일."""
    key_path = ROOT / ".secrets" / "pengport-updater.key"
    if not key_path.exists():
        raise FileNotFoundError(f"서명 키 없음: {key_path}")
    env = os.environ.copy()
    env["TAURI_SIGNING_PRIVATE_KEY"] = key_path.read_text(encoding="utf-8").strip()
    env.setdefault("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "")  # 비밀번호 없는 키
    run_env(
        [PNPM, "run", "tauri", "signer", "sign", str(path)],
        cwd=ROOT / "player-launcher",
        env=env,
    )
    sig_path = path.with_name(path.name + ".sig")
    if not sig_path.is_file():
        raise FileNotFoundError(f"서명 실패: {sig_path} 생성 안 됨")
    return sig_path


def collect_and_sign_raw_exe(release_dir: Path, exe: Path, app_version: str) -> tuple[Path, Path]:
    """Tauri exe를 release_dir 에 버전 붙은 이름으로 복사하고 updater 서명 키로
    직접 서명한다.

    자체 업데이터(`commands/self_update.rs`)는 `latest.json`이 가리키는 바이트를
    그대로 실행 파일 자리에 rename-to-delete 로 앉힌다. **2026-08 실제 사고**: 한때
    빌드가 NSIS installer도 같이 만들었는데, 그걸 구분 없이 `write_latest_json`에
    넘겼다가 0.2.0→0.2.1 자체 업데이트가 실사용자의 PengPort.exe 를 설치 마법사로
    바꿔버림(다행히 rename-to-delete 가 원본을 `PengPort.old.exe`로 보존해둬서 복구는
    됨) — 이 사고가 NSIS installer를 완전히 없애고 포터블 전용으로 간 계기(위 모듈
    docstring 참고). (exe_out, sig_out) 반환."""
    exe_out = release_dir / f"PengPort-{app_version}.exe"
    shutil.copy2(exe, exe_out)
    sig_out = sign_file(exe_out)
    return exe_out, sig_out


def write_latest_json(release_dir: Path, asset: Path, sig: Path, app_version: str) -> Path:
    """Tauri updater 가 읽는 manifest 생성.
    endpoints (tauri.conf.json) 는 `latest.json` 을 가리키고,
    여기 포함된 url 로 클라가 asset(exe — `collect_and_sign_raw_exe` 참고)을 받음.
    URL 인코딩이 필요한 문자는 한 번에 quote 처리 (파일명에 공백 등 들어가도 안전)."""
    from urllib.parse import quote

    signature = sig.read_text(encoding="utf-8").strip()
    asset_url = f"{UPDATES_BASE_URL}/{quote(asset.name)}"
    manifest = {
        "version": app_version,
        "notes": f"PengPort v{app_version}",
        "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": {
            "windows-x86_64": {
                "signature": signature,
                "url": asset_url,
            },
        },
    }
    out = release_dir / "latest.json"
    out.write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding="utf-8")
    return out


def run_env(cmd: list[str], cwd: Path | None = None, env: dict | None = None) -> None:
    print(f"$ {' '.join(str(c) for c in cmd)}  (cwd={cwd or ROOT})")
    subprocess.run(cmd, cwd=cwd or ROOT, check=True, env=env)


def stage_bundle(staging: Path, exe: Path) -> None:
    """staging 폴더에 최종 배포 구조를 구성 — PengPort.exe 하나뿐(원클릭 컨셉상
    별도 안내문 없이도 실행만 하면 되게, 2026-08 사용자 확인)."""
    if staging.exists():
        shutil.rmtree(staging)
    staging.mkdir(parents=True)

    # Tauri exe → 사용자에게 보이는 이름으로 복사
    shutil.copy2(exe, staging / "PengPort.exe")


def zip_staging(staging: Path, out: Path) -> Path:
    """주어진 path 로 zip 생성. 실제 작성된 path 반환.

    Windows 에서 기존 zip 이 Defender 스캔/Explorer 미리보기/Bandizip 등에
    의해 짧게 잠기는 경우가 있어, unlink 가 실패하면 짧은 backoff 로 재시도한다.
    그래도 안 풀리면 timestamp 가 붙은 새 이름으로 작성해 워크플로우를 막지 않는다."""
    import time

    if out.exists():
        # 0.3s × 8회 = 약 2.4s 까지 대기 (Defender 짧은 스캔이면 충분)
        for attempt in range(8):
            try:
                out.unlink()
                break
            except PermissionError:
                if attempt < 7:
                    time.sleep(0.3)
                    continue
                # 끝까지 잠금 해제 안 되면 timestamp 이름으로 fallback.
                ts = datetime.now().strftime("%Y%m%d-%H%M%S")
                fallback = out.with_name(f"{out.stem}-{ts}{out.suffix}")
                print(
                    f"[WARN] 기존 {out.name} 의 잠금이 풀리지 않아 "
                    f"{fallback.name} 으로 작성합니다.",
                    file=sys.stderr,
                )
                out = fallback

    out.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for path in staging.rglob("*"):
            z.write(path, path.relative_to(staging))
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", default="v3", help="Portable zip 라벨 (배포 세대 구분용)")
    ap.add_argument("--skip-build", action="store_true", help="Tauri 빌드 생략 (기존 산출물 재사용)")
    ap.add_argument("--skip-release", action="store_true",
                    help="release assets(latest.json 등) 생성 생략 (portable zip 만)")
    args = ap.parse_args()

    # Tauri 의 실제 앱 버전은 tauri.conf.json 의 version 필드가 출처.
    app_version = read_tauri_conf()["version"]

    out_dir = ROOT / "client" / "build"
    staging = out_dir / f"staging-{args.version}"
    zip_path = out_dir / f"PengPort-{args.version}.zip"
    release_dir = out_dir / f"release-{args.version}"

    exe = (
        ROOT / "target" / "release" / "pengport.exe"
        if args.skip_build
        else build_tauri()
    )
    if not exe.exists():
        print(f"[FAIL] 빌드 산출물 없음: {exe}", file=sys.stderr)
        return 1

    # 1) Portable zip (PengPort.exe 하나, 최초 배포용)
    stage_bundle(staging, exe)
    zip_path = zip_staging(staging, zip_path)  # 잠금 시 timestamp 이름으로 fallback 가능
    size_mb = zip_path.stat().st_size / (1024 * 1024)
    print(f"\n[OK] portable zip: {zip_path} ({size_mb:.1f} MB)")

    # 2) Release assets (자동 업데이트용)
    if args.skip_release:
        print("[skip] release assets (--skip-release)")
        return 0

    # 깨끗한 release_dir 로 시작(이전 빌드 잔재가 SCP 로 함께 올라가는 것 방지).
    if release_dir.exists():
        shutil.rmtree(release_dir)
    release_dir.mkdir(parents=True)

    try:
        asset, sig = collect_and_sign_raw_exe(release_dir, exe, app_version)
    except FileNotFoundError as e:
        print(f"[WARN] release assets 수집 실패: {e}", file=sys.stderr)
        print("       portable zip 만 생성됨. 자동 업데이트 배포는 불가.", file=sys.stderr)
        return 0

    manifest = write_latest_json(release_dir, asset, sig, app_version)
    asset_mb = asset.stat().st_size / (1024 * 1024)
    print(f"[OK] update asset (raw exe): {asset} ({asset_mb:.1f} MB)")
    print(f"[OK] signature:              {sig}")
    print(f"[OK] latest.json:            {manifest}")
    # 실제 GitHub Release 업로드는 release.yml의 별도 단계(gh release create/upload)가
    # 담당 — 이 스크립트는 빌드/서명까지만, laptop에서 --skip-build 없이 그냥 돌려도
    # 아무 데도 안 올라가고 로컬 산출물만 남는다(안전).
    return 0


if __name__ == "__main__":
    sys.exit(main())

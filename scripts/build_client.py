#!/usr/bin/env python3
"""
build_client.py — PengPort 배포 번들 빌드.

산출물:
1) Portable zip   (`client/build/PengPort-{version}.zip`)
     PengPort.exe                # Tauri release 바이너리 — 원클릭 컨셉상 안내문 없음
2) Release assets (`client/build/release-{version}/`)
     PengPort-{X.Y.Z}.exe            # raw exe(NSIS 래핑 안 됨) — 자동 업데이트(자체
                                      # rename-to-delete 업데이터)가 받는 실제 대상
     PengPort-{X.Y.Z}.exe.sig        # 위 raw exe의 minisign 서명
     latest.json                     # Tauri updater manifest, url → 위 raw exe
                                      # (https://pengdoll.duckdns.org/updates/...)
     PengPort_{X.Y.Z}_x64-setup.exe     # NSIS installer — 수동 배포/신규 사용자용일 뿐,
     PengPort_{X.Y.Z}_x64-setup.exe.sig # latest.json 대상 아님(자동 업데이트가 이걸
                                         # 받으면 다음 실행 시 installer 가 열려버림 —
                                         # 2026-08 실제 사고, collect_and_sign_raw_exe
                                         # 문서 참고)

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

# Self-hosted update endpoint. installer/sig/latest.json 모두 같은 디렉터리에 배치.
# Caddy 가 file_server 로 정적 서빙 (public — PSP 정신상 client 는 software,
# instance-agnostic 이라 누구나 다운로드. instance 접근만 EVENTS_TOKEN 으로 보호).
# instance 별로 도메인이 다르므로 환경변수 override 가능 (GH Actions / 다른 instance fork).
UPDATES_BASE_URL = os.environ.get(
    "PENGPORT_UPDATES_BASE_URL",
    "https://pengdoll.duckdns.org/updates",
)

# 자동 업로드 설정.
# - PENGPORT_DEPLOY_SSH_HOST: SCP 대상 ssh config alias. 비어있으면 upload skip.
#   본격 release 는 GH Actions tag push (release.yml) 로만 — laptop 은 build-only.
# - PENGPORT_DEPLOY_UPDATES_DIR: 원격 호스트의 caddy mount source 디렉토리 경로.
#   pengport-updater user 의 home (`~`) 기준 — 격리된 sftp-only 계정.
SSH_HOST = os.environ.get("PENGPORT_DEPLOY_SSH_HOST", "")
REMOTE_UPDATES_DIR = os.environ.get("PENGPORT_DEPLOY_UPDATES_DIR", "~/updates")


def read_tauri_conf() -> dict:
    return json.loads(TAURI_CONF.read_text(encoding="utf-8"))


def run(cmd: list[str], cwd: Path | None = None) -> None:
    print(f"$ {' '.join(str(c) for c in cmd)}  (cwd={cwd or ROOT})")
    subprocess.run(cmd, cwd=cwd or ROOT, check=True)


def build_tauri() -> Path:
    """Tauri release 빌드 실행 (NSIS installer + 서명 포함). 결과 exe 경로 반환.
    서명 키가 `.secrets/pengport-updater.key` 에 있으면 자동으로 서명 (.sig 생성)."""
    env = os.environ.copy()
    key_path = ROOT / ".secrets" / "pengport-updater.key"
    if key_path.exists():
        env["TAURI_SIGNING_PRIVATE_KEY"] = key_path.read_text(encoding="utf-8").strip()
        env["TAURI_SIGNING_PRIVATE_KEY_PASSWORD"] = ""  # 비밀번호 없는 키
        print(f"[signing] private key loaded from {key_path}")
    else:
        print(f"[signing] no key at {key_path} — unsigned build (NSIS .sig 생성 안됨)")

    run_env([PNPM, "install", "--frozen-lockfile"], cwd=ROOT / "player-launcher", env=env)
    # NSIS 만 빌드 (updater 는 NSIS 기반). MSI 는 필요 시 별도 target 로 추가.
    run_env([PNPM, "run", "tauri", "build", "--bundles", "nsis"],
            cwd=ROOT / "player-launcher", env=env)
    # Cargo crate 이름이 'pengport' 이라 산출물도 같은 이름.
    exe = ROOT / "target" / "release" / "pengport.exe"
    if not exe.exists():
        raise FileNotFoundError(f"Tauri 빌드 산출물을 찾을 수 없습니다: {exe}")
    return exe


def upload_release(release_dir: Path) -> None:
    """release_dir 의 파일들을 deploy host 의 updates/ 디렉토리로 SCP.

    deploy 대상은 SFTP-only 격리 계정 (예: pengport-updater) — restrict + ForceCommand
    internal-sftp 로 강제되어 있어 ssh shell / 임의 명령 실행 불가.
    따라서 stale 정리도 SCP 로 새 파일을 업로드하면 동일 이름은 자연 덮어씌워지고,
    이전 productName 잔재 등은 별도 명령(ssh) 으로 못 지우므로 사용자가 수동 정리.

    SSH_HOST 가 비어있으면 upload 자체를 skip (laptop 의 build-only 모드)."""
    if not SSH_HOST:
        print("[skip] upload: PENGPORT_DEPLOY_SSH_HOST 환경변수 없음")
        print("       laptop 에서는 build-only — release 는 GH Actions tag push 로만 진행")
        return

    files = sorted(release_dir.iterdir())
    if not files:
        print(f"[skip] upload: {release_dir} 비어있음")
        return

    # SCP 는 single shot 으로 여러 파일 가능. 절대경로 보장.
    # OpenSSH 9+ 의 scp 는 sftp protocol backend 로 작동 → ForceCommand internal-sftp 와 호환.
    cmd = ["scp", *[str(f) for f in files], f"{SSH_HOST}:{REMOTE_UPDATES_DIR}/"]
    print(f"$ {' '.join(cmd)}")
    subprocess.run(cmd, check=True)
    print(f"[OK] uploaded {len(files)} files to {SSH_HOST}:{REMOTE_UPDATES_DIR}/")


def collect_release_assets(release_dir: Path, app_version: str) -> tuple[Path, Path]:
    """현재 tauri.conf.json 의 (productName, version) 조합으로 정확한 NSIS 산출물을
    찾아 release 디렉터리로 복사. 이전 빌드 잔재가 NSIS 폴더에 남아있어도 정확한
    파일을 매칭하므로 안전.
    (installer_out, sig_out) 반환."""
    nsis_dir = ROOT / "target" / "release" / "bundle" / "nsis"
    if not nsis_dir.exists():
        raise FileNotFoundError(f"NSIS 빌드 디렉터리 없음: {nsis_dir}")

    # Tauri NSIS 산출물 명명 규칙: `<productName>_<version>_x64-setup.exe`
    product_name = read_tauri_conf()["productName"]
    installer_src = nsis_dir / f"{product_name}_{app_version}_x64-setup.exe"
    if not installer_src.is_file():
        # 실패 시 같은 폴더의 다른 산출물 목록도 함께 보여 디버깅 도움.
        siblings = sorted(nsis_dir.glob("*_x64-setup.exe"))
        raise FileNotFoundError(
            f"NSIS installer 없음: {installer_src.name}\n"
            f"NSIS 폴더의 산출물: {[p.name for p in siblings]}\n"
            f"→ Tauri 빌드가 정상 완료됐는지, productName/version 이 일치하는지 확인."
        )
    # Tauri 는 `<installer>.exe.sig` 형태로 서명 파일을 나란히 둠.
    sig_src = installer_src.parent / (installer_src.name + ".sig")
    if not sig_src.is_file():
        raise FileNotFoundError(
            f"서명 파일 없음: {sig_src}\n"
            "→ `.secrets/pengport-updater.key` 가 있는지 확인하세요."
        )

    # 깨끗한 release_dir 로 시작 (이전 빌드의 다른 이름 산출물이 SCP 로 함께 올라가는 것 방지).
    if release_dir.exists():
        shutil.rmtree(release_dir)
    release_dir.mkdir(parents=True)
    installer_out = release_dir / installer_src.name
    sig_out = release_dir / sig_src.name
    shutil.copy2(installer_src, installer_out)
    shutil.copy2(sig_src, sig_out)
    return installer_out, sig_out


def sign_file(path: Path) -> Path:
    """`path`를 updater 서명 키로 minisign 서명 — `tauri signer sign`(Tauri CLI,
    NSIS 자동서명과 정확히 같은 포맷)을 그대로 재사용해서 `<path>.sig`를 만든다.
    반환값은 그 서명 파일 경로.

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
    """raw Tauri exe(NSIS 래핑 안 됨)를 release_dir 에 버전 붙은 이름으로 복사하고
    updater 서명 키로 직접 서명한다.

    자체 업데이터(`commands/self_update.rs`)는 `latest.json`이 가리키는 바이트를
    그대로 실행 파일 자리에 rename-to-delete 로 앉힌다 — 그 자산이 NSIS 인스톨러면
    다음 실행 시 PengPort.exe 가 사실은 설치 마법사가 되어버린다. **2026-08 실제
    사고**: 이 구분 없이 NSIS installer 를 그대로 `write_latest_json`에 넘겼다가
    0.2.0→0.2.1 자체 업데이트가 실사용자의 PengPort.exe 를 설치 마법사로 바꿔버림
    (다행히 rename-to-delete 가 원본을 `PengPort.old.exe`로 보존해둬서 복구는 됨).
    portable 모델에서 자체 업데이트 자산은 항상 이 raw exe여야 한다 — NSIS installer
    는 (아직 남아있다면) 별개의 수동 설치용 산출물일 뿐, latest.json 대상이 아니다.
    (exe_out, sig_out) 반환."""
    exe_out = release_dir / f"PengPort-{app_version}.exe"
    shutil.copy2(exe, exe_out)
    sig_out = sign_file(exe_out)
    return exe_out, sig_out


def write_latest_json(release_dir: Path, asset: Path, sig: Path, app_version: str) -> Path:
    """Tauri updater 가 읽는 manifest 생성.
    endpoints (tauri.conf.json) 는 `latest.json` 을 가리키고,
    여기 포함된 url 로 클라가 asset(raw exe — `collect_and_sign_raw_exe` 참고, NSIS
    인스톨러 아님)을 받음. URL 인코딩이 필요한 문자는 한 번에 quote 처리 (파일명에
    공백 등 들어가도 안전)."""
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
                    help="NSIS installer / latest.json 생성 생략 (portable zip 만)")
    ap.add_argument("--no-upload", action="store_true",
                    help="release assets 의 Oracle 자동 SCP 업로드 생략")
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

    # NSIS installer 는 latest.json 대상이 아니다(아래 collect_and_sign_raw_exe
    # 문서 참고) — 수동 배포/신규 사용자용 산출물로만 같이 챙긴다. 이 수집이 release_dir
    # 를 먼저 비우고 새로 만드므로(collect_release_assets 참고) raw exe 수집보다 먼저.
    try:
        installer, _installer_sig = collect_release_assets(release_dir, app_version)
        inst_mb = installer.stat().st_size / (1024 * 1024)
        print(f"[OK] NSIS installer (수동 배포용, 자동 업데이트 대상 아님): {installer} ({inst_mb:.1f} MB)")
    except FileNotFoundError as e:
        print(f"[WARN] NSIS installer 수집 실패(자동 업데이트엔 영향 없음, 계속 진행): {e}", file=sys.stderr)
        release_dir.mkdir(parents=True, exist_ok=True)

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

    # 3) Oracle 자동 업로드
    if args.no_upload:
        print("[skip] upload (--no-upload). 수동 업로드:")
        print(f"  scp {release_dir}/* {SSH_HOST}:{REMOTE_UPDATES_DIR}/")
    else:
        try:
            upload_release(release_dir)
            print(f"\n[DONE] 자동 업데이트 배포 완료. endpoint: {UPDATES_BASE_URL}/latest.json")
        except subprocess.CalledProcessError as e:
            print(f"[WARN] 업로드 실패 (exit {e.returncode}). 수동 업로드 필요.", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  checkForUpdate,
  getUpdateToken,
  getUpdateTokenSource,
  maskToken,
  setUpdateToken,
  type TokenSource,
  type UpdateInfo,
} from "@/lib/updater";
import {
  detectPrism,
  downloadPrism,
  removeBundledPrism,
  setPrismOverride,
  type PrismLocation,
  type PrismSource,
} from "@/lib/api";
import {
  pspListTrusts,
  pspRevokeTrust,
  type TrustEntryDto,
} from "@/lib/psp";

type CheckState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "up_to_date"; currentVersion: string | null }
  | { kind: "available"; info: UpdateInfo }
  | { kind: "installing" }
  | { kind: "error"; message: string };

type TokenState = {
  current: string;
  source: TokenSource;
  editing: boolean;
  draft: string;
  saving: boolean;
  error: string | null;
};

const initialTokenState: TokenState = {
  current: "",
  source: "none",
  editing: false,
  draft: "",
  saving: false,
  error: null,
};

export default function Settings() {
  const [state, setState] = useState<CheckState>({ kind: "idle" });
  const [tokenState, setTokenState] = useState<TokenState>(initialTokenState);
  // 토큰 UI 는 평소엔 숨김. 인증 실패 또는 토큰 부재 시에만 노출.
  // 토큰 회전 후에는 자동 update check 가 실패 → 노출 → 사용자가 새 토큰 입력.
  const [showTokenSection, setShowTokenSection] = useState(false);

  const refreshToken = async () => {
    const [current, source] = await Promise.all([
      getUpdateToken(),
      getUpdateTokenSource(),
    ]);
    setTokenState((s) => ({ ...s, current, source }));
    if (!current || source === "none") setShowTokenSection(true);
  };

  useEffect(() => {
    refreshToken().catch((e) => {
      setTokenState((s) => ({ ...s, error: String(e) }));
      setShowTokenSection(true);
    });
    // 토큰 노출 결정용 silent check. 에러면 토큰 섹션 노출.
    checkForUpdate().catch(() => setShowTokenSection(true));
  }, []);

  const onCheck = async () => {
    setState({ kind: "checking" });
    try {
      const info = await checkForUpdate();
      if (info.available) {
        setState({ kind: "available", info });
      } else {
        setState({ kind: "up_to_date", currentVersion: info.currentVersion });
      }
    } catch (e) {
      setState({ kind: "error", message: String(e) });
      // 인증 / 네트워크 에러 → 사용자가 토큰 점검할 수 있게 노출.
      setShowTokenSection(true);
    }
  };

  const onInstall = async () => {
    if (state.kind !== "available" || !state.info.install) return;
    setState({ kind: "installing" });
    try {
      await state.info.install();
      // relaunch() 이후로 이 줄은 실행 안 됨
    } catch (e) {
      setState({ kind: "error", message: String(e) });
    }
  };

  const onTokenSave = async () => {
    setTokenState((s) => ({ ...s, saving: true, error: null }));
    try {
      await setUpdateToken(tokenState.draft);
      await refreshToken();
      setTokenState((s) => ({
        ...s,
        editing: false,
        draft: "",
        saving: false,
      }));
    } catch (e) {
      setTokenState((s) => ({ ...s, saving: false, error: String(e) }));
    }
  };

  const onTokenReset = async () => {
    setTokenState((s) => ({ ...s, saving: true, error: null }));
    try {
      // 빈 문자열 저장 = 사용자 저장값 삭제 → 임베드 기본값으로 fallback
      await setUpdateToken("");
      await refreshToken();
      setTokenState((s) => ({
        ...s,
        editing: false,
        draft: "",
        saving: false,
      }));
    } catch (e) {
      setTokenState((s) => ({ ...s, saving: false, error: String(e) }));
    }
  };

  return (
    <div className="space-y-6 p-8">
      <h2 className="text-2xl font-semibold">설정</h2>

      <section className="space-y-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5">
        <h3 className="text-sm font-medium text-neutral-200">업데이트</h3>

        <div className="flex items-center gap-3">
          <Button
            size="sm"
            onClick={onCheck}
            disabled={state.kind === "checking" || state.kind === "installing"}
          >
            {state.kind === "checking" ? "확인 중..." : "업데이트 확인"}
          </Button>

          {state.kind === "available" && (
            <Button size="sm" onClick={onInstall}>
              새 버전 설치 ({state.info.version})
            </Button>
          )}
        </div>

        {state.kind === "up_to_date" && (
          <p className="text-xs text-neutral-400">
            최신 버전입니다
            {state.currentVersion ? ` (v${state.currentVersion})` : ""}.
          </p>
        )}

        {state.kind === "available" && state.info.body && (
          <pre className="mt-2 whitespace-pre-wrap rounded bg-neutral-950 p-3 text-xs text-neutral-300">
            {state.info.body}
          </pre>
        )}

        {state.kind === "installing" && (
          <p className="text-xs text-neutral-400">
            다운로드 + 설치 중... 완료되면 자동으로 재시작됩니다.
          </p>
        )}

        {state.kind === "error" && (
          <p className="text-xs text-red-300" title={state.message}>
            실패:{" "}
            {state.message.length > 100
              ? state.message.slice(0, 100) + "..."
              : state.message}
          </p>
        )}
      </section>

      <PrismSection />

      <TrustSection />

      {showTokenSection && (
      <section className="space-y-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5">
        <div className="flex items-baseline justify-between">
          <h3 className="text-sm font-medium text-neutral-200">
            업데이트 토큰
          </h3>
          <span className="text-[11px] text-neutral-500">
            출처: {sourceLabel(tokenState.source)}
          </span>
        </div>

        <p className="text-xs text-neutral-400">
          업데이트 서버 인증용 토큰입니다. 서버에서 토큰을 회전했다면 새 값으로
          변경하세요.
        </p>

        {!tokenState.editing ? (
          <div className="flex items-center gap-3">
            <code className="flex-1 rounded bg-neutral-950 px-3 py-2 text-xs text-neutral-300">
              {maskToken(tokenState.current)}
            </code>
            <Button
              size="sm"
              variant="outline"
              onClick={() =>
                setTokenState((s) => ({
                  ...s,
                  editing: true,
                  draft: "",
                  error: null,
                }))
              }
            >
              변경
            </Button>
            {tokenState.source === "saved" && (
              <Button
                size="sm"
                variant="outline"
                onClick={onTokenReset}
                disabled={tokenState.saving}
              >
                초기화
              </Button>
            )}
          </div>
        ) : (
          <div className="space-y-2">
            <input
              type="text"
              value={tokenState.draft}
              onChange={(e) =>
                setTokenState((s) => ({ ...s, draft: e.target.value }))
              }
              placeholder="새 토큰 붙여넣기"
              className="w-full rounded bg-neutral-950 px-3 py-2 font-mono text-xs text-neutral-100 outline-none ring-1 ring-neutral-800 focus:ring-neutral-600"
              autoFocus
              spellCheck={false}
            />
            <div className="flex gap-2">
              <Button
                size="sm"
                onClick={onTokenSave}
                disabled={tokenState.saving || !tokenState.draft.trim()}
              >
                {tokenState.saving ? "저장 중..." : "저장"}
              </Button>
              <Button
                size="sm"
                variant="outline"
                onClick={() =>
                  setTokenState((s) => ({
                    ...s,
                    editing: false,
                    draft: "",
                    error: null,
                  }))
                }
                disabled={tokenState.saving}
              >
                취소
              </Button>
            </div>
          </div>
        )}

        {tokenState.error && (
          <p className="text-xs text-red-300">실패: {tokenState.error}</p>
        )}
      </section>
      )}
    </div>
  );
}

function sourceLabel(s: TokenSource): string {
  switch (s) {
    case "saved":
      return "사용자 저장값";
    case "embedded":
      return "빌드 기본값";
    case "none":
      return "(없음)";
  }
}

function prismSourceLabel(s: PrismSource): string {
  switch (s) {
    case "user":
      return "사용자 지정";
    case "env":
      return "환경변수 (개발)";
    case "portable":
      return "exe 옆 폴더";
    case "bundled":
      return "전용 (자동 다운로드)";
    case "system":
      return "시스템 설치본";
    case "path":
      return "PATH 발견";
  }
}

type PrismOp =
  | { kind: "idle" }
  | { kind: "downloading" }
  | { kind: "removing" }
  | { kind: "error"; message: string };

function PrismSection() {
  const [location, setLocation] = useState<PrismLocation | null | undefined>(
    undefined,
  );
  const [op, setOp] = useState<PrismOp>({ kind: "idle" });

  const refresh = async () => {
    try {
      setLocation(await detectPrism());
    } catch (e) {
      setLocation(null);
      setOp({ kind: "error", message: String(e) });
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const onPickFolder = async () => {
    setOp({ kind: "idle" });
    try {
      // dialog plugin 으로 폴더 선택 (네이티브 OS 다이얼로그).
      const { open } = await import("@tauri-apps/plugin-dialog");
      const picked = await open({
        directory: true,
        multiple: false,
        title: "prismlauncher.exe 가 있는 폴더 선택",
      });
      if (!picked || typeof picked !== "string") return;
      const next = await setPrismOverride(picked);
      setLocation(next);
    } catch (e) {
      setOp({ kind: "error", message: String(e) });
    }
  };

  const onClearOverride = async () => {
    setOp({ kind: "idle" });
    try {
      const next = await setPrismOverride("");
      setLocation(next);
    } catch (e) {
      setOp({ kind: "error", message: String(e) });
    }
  };

  const onDownload = async () => {
    setOp({ kind: "downloading" });
    try {
      await downloadPrism();
      await refresh();
      setOp({ kind: "idle" });
    } catch (e) {
      setOp({ kind: "error", message: String(e) });
    }
  };

  const onRemoveBundled = async () => {
    setOp({ kind: "removing" });
    try {
      const next = await removeBundledPrism();
      setLocation(next);
      setOp({ kind: "idle" });
    } catch (e) {
      setOp({ kind: "error", message: String(e) });
    }
  };

  if (location === undefined) {
    return (
      <section className="space-y-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5">
        <h3 className="text-sm font-medium text-neutral-200">PrismLauncher</h3>
        <p className="text-xs text-neutral-400">탐지 중...</p>
      </section>
    );
  }

  return (
    <section className="space-y-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5">
      <div className="flex items-baseline justify-between">
        <h3 className="text-sm font-medium text-neutral-200">PrismLauncher</h3>
        {location && (
          <span className="text-[11px] text-neutral-500">
            출처: {prismSourceLabel(location.source)}
          </span>
        )}
      </div>

      {location ? (
        <>
          <div className="space-y-1 text-xs text-neutral-400">
            <div>
              실행 파일:{" "}
              <code className="text-neutral-300">{location.exe}</code>
            </div>
            <div>
              데이터 폴더:{" "}
              <code className="text-neutral-300">{location.data_dir}</code>
            </div>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button size="sm" variant="outline" onClick={onPickFolder}>
              다른 폴더 지정
            </Button>
            {location.source === "user" && (
              <Button size="sm" variant="outline" onClick={onClearOverride}>
                지정 해제 (자동 탐색 복귀)
              </Button>
            )}
            {location.source === "bundled" && (
              <Button
                size="sm"
                variant="outline"
                onClick={onRemoveBundled}
                disabled={op.kind === "removing"}
              >
                {op.kind === "removing" ? "삭제 중..." : "전용 Prism 삭제"}
              </Button>
            )}
          </div>
        </>
      ) : (
        <>
          <p className="text-xs text-neutral-400">
            PrismLauncher 를 찾을 수 없습니다. 자동 다운로드하거나 폴더를 직접
            지정하세요.
          </p>
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              onClick={onDownload}
              disabled={op.kind === "downloading"}
            >
              {op.kind === "downloading"
                ? "다운로드 중..."
                : "자동 다운로드"}
            </Button>
            <Button size="sm" variant="outline" onClick={onPickFolder}>
              폴더 직접 지정
            </Button>
          </div>
        </>
      )}

      {op.kind === "error" && (
        <p className="text-xs text-red-300" title={op.message}>
          실패:{" "}
          {op.message.length > 200
            ? op.message.slice(0, 200) + "..."
            : op.message}
        </p>
      )}
    </section>
  );
}

// ====== 신뢰 관리 (PSP 3-tier trust) ======

function TrustSection() {
  const [entries, setEntries] = useState<TrustEntryDto[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const list = await pspListTrusts();
      list.sort((a, b) => b.trusted_at - a.trusted_at);
      setEntries(list);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const onRevoke = async (entry: TrustEntryDto) => {
    try {
      await pspRevokeTrust(entry.subject_kind, entry.subject_id);
      await refresh();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <section className="space-y-3 rounded-lg border border-neutral-800 bg-neutral-900/60 p-5">
      <div className="flex items-baseline justify-between">
        <h3 className="text-sm font-medium text-neutral-200">신뢰 관리</h3>
        <span className="text-[11px] text-neutral-500">
          PSP 3-tier 신뢰 목록 (TOFU)
        </span>
      </div>

      <p className="text-xs text-neutral-400">
        외부 앱 실행, 인스턴스 추가, 서비스 권한 등 사용자가 명시적으로 동의한
        신뢰 항목입니다. 철회 시 다음 동일 호출에서 다시 확인 메시지가 표시됩니다.
      </p>

      {error && <p className="text-xs text-red-300">실패: {error}</p>}

      {entries === null ? (
        <p className="text-xs text-neutral-500">불러오는 중...</p>
      ) : entries.length === 0 ? (
        <p className="text-xs text-neutral-500">신뢰 항목이 없습니다.</p>
      ) : (
        <ul className="space-y-2">
          {entries.map((e) => (
            <li
              key={`${e.subject_kind}|${e.subject_id}`}
              className="flex items-center justify-between gap-3 rounded border border-neutral-800 bg-neutral-950/40 px-3 py-2 text-xs"
            >
              <div className="min-w-0 flex-1">
                <div className="truncate text-neutral-200">{e.display}</div>
                <div className="mt-0.5 truncate font-mono text-[10.5px] text-neutral-500">
                  {labelForKind(e.subject_kind)} · {e.subject_id}
                </div>
              </div>
              <Button size="sm" variant="outline" onClick={() => onRevoke(e)}>
                철회
              </Button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function labelForKind(kind: string): string {
  if (kind === "third_party.prism-launcher") return "Prism 외부 앱";
  if (kind.startsWith("third_party.")) return `외부 앱 (${kind.slice(12)})`;
  if (kind === "instance") return "인스턴스";
  if (kind.startsWith("service.")) return "서비스";
  return kind;
}

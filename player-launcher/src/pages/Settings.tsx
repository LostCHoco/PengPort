import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { checkForUpdate, type UpdateInfo } from "@/lib/updater";
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

export default function Settings() {
  const [state, setState] = useState<CheckState>({ kind: "idle" });

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

      <TrustSection />
    </div>
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

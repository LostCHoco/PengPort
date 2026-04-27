// PSP 3-tier 신뢰 동의 dialog.
//
// ActionOutcome::NeedsConfirm 을 받았을 때 부모(Library/PspLibrary) 가 띄움.
// 사용자가 "허용" 누르면 psp_trust → invoke 재시도, "거절" 이면 그냥 닫기.
//
// trust_kind 별 본문 표현 분기:
// - `third_party.{app_id}` — 외부 앱 실행 신뢰 (host:port + packwiz_url 등)
// - `instance` — 인스턴스 신뢰 (Tier 1, 미래)
// - `service.{...}` — service 별 신뢰 (Tier 2, 미래)

import { useEffect, useRef } from "react";
import { Button } from "@/components/ui/button";

export interface ConsentRequest {
  trust_kind: string;
  subject_id: string;
  display: string;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  details: any;
}

interface Props {
  /** null 이면 dialog 닫힘 상태. */
  request: ConsentRequest | null;
  /** "허용" — 부모가 psp_trust 호출 후 invoke 재시도. */
  onAllow: (req: ConsentRequest) => void;
  /** "거절" — dialog 만 닫음. */
  onDeny: () => void;
  /** "허용" 후 처리 중 (psp_trust + invoke 재시도) — 버튼 비활성. */
  processing?: boolean;
}

export function ConsentDialog({ request, onAllow, onDeny, processing }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);

  // ESC 닫기. 처리 중일 때는 무시.
  useEffect(() => {
    if (!request) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !processing) onDeny();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [request, onDeny, processing]);

  // 첫 포커스 — "거절" (안전한 기본값).
  useEffect(() => {
    if (!request) return;
    cardRef.current
      ?.querySelector<HTMLButtonElement>("[data-consent-deny]")
      ?.focus();
  }, [request]);

  if (!request) return null;

  const handleBackdrop = () => {
    if (!processing) onDeny();
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onClick={handleBackdrop}
      role="dialog"
      aria-modal="true"
      aria-labelledby="consent-title"
    >
      <div
        ref={cardRef}
        className="w-full max-w-md rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 id="consent-title" className="text-lg font-semibold text-neutral-50">
          {titleFor(request.trust_kind)}
        </h3>

        <div className="mt-3 space-y-3 text-sm text-neutral-300">
          <p>
            <span className="font-medium text-neutral-100">{request.display}</span>{" "}
            {leadFor(request.trust_kind)}
          </p>

          <DetailsBlock request={request} />

          <p className="text-xs text-neutral-500">
            허용하면 동일 호출에 대해 다시 묻지 않습니다. 변경 발생 시 (예:
            모드팩 출처가 바뀌면) 다시 확인 메시지가 표시됩니다. Settings 에서
            언제든 신뢰를 취소할 수 있습니다.
          </p>
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <Button
            data-consent-deny
            variant="outline"
            size="sm"
            disabled={processing}
            onClick={onDeny}
            className="cursor-pointer"
          >
            거절
          </Button>
          <Button
            size="sm"
            disabled={processing}
            onClick={() => onAllow(request)}
            className="min-w-[80px] cursor-pointer"
          >
            {processing ? (
              <span className="inline-flex items-center gap-1.5">
                <Spinner />
                처리 중
              </span>
            ) : (
              "허용"
            )}
          </Button>
        </div>
      </div>
    </div>
  );
}

function titleFor(trustKind: string): string {
  if (trustKind.startsWith("third_party.")) {
    return "외부 앱 실행 허용";
  }
  if (trustKind === "instance") {
    return "인스턴스 추가";
  }
  if (trustKind.startsWith("service.")) {
    return "서비스 추가";
  }
  return "동의 필요";
}

function leadFor(trustKind: string): string {
  if (trustKind.startsWith("third_party.")) {
    return "에 외부 앱 (PrismLauncher 등) 으로 접속합니다.";
  }
  if (trustKind === "instance") {
    return "에 연결합니다.";
  }
  if (trustKind.startsWith("service.")) {
    return "에 접근합니다.";
  }
  return "에 대해 동의가 필요합니다.";
}

function DetailsBlock({ request }: { request: ConsentRequest }) {
  if (request.trust_kind === "third_party.prism-launcher") {
    return <PrismLauncherDetails details={request.details} />;
  }
  // 알 수 없는 trust_kind — 디버깅용 raw 표시.
  return (
    <pre className="overflow-x-auto rounded bg-neutral-950/60 p-2 text-[11px] text-neutral-400">
      {JSON.stringify(request.details, null, 2)}
    </pre>
  );
}

interface PrismLauncherConsentDetails {
  app_id: string;
  host: string;
  port: number;
  version: string;
  loader: string;
  loader_version?: string | null;
  packwiz_url?: string | null;
  install_hint?: { name: string; homepage?: string | null } | null;
}

function PrismLauncherDetails({ details }: { details: PrismLauncherConsentDetails }) {
  return (
    <dl className="grid grid-cols-[7rem_1fr] gap-x-3 gap-y-1.5 rounded border border-neutral-800 bg-neutral-950/40 px-3 py-2.5 text-xs">
      <dt className="text-neutral-500">접속 대상</dt>
      <dd className="font-mono text-neutral-200">
        {details.host}:{details.port}
      </dd>

      <dt className="text-neutral-500">Minecraft</dt>
      <dd className="text-neutral-200">
        {details.version}
        {" · "}
        {labelForLoader(details.loader)}
        {details.loader_version ? ` ${details.loader_version}` : ""}
      </dd>

      {details.packwiz_url && (
        <>
          <dt className="text-neutral-500">모드팩 출처</dt>
          <dd className="break-all font-mono text-[10.5px] text-neutral-300">
            {details.packwiz_url}
          </dd>
        </>
      )}

      {details.install_hint?.name && (
        <>
          <dt className="text-neutral-500">실행 도구</dt>
          <dd className="text-neutral-200">{details.install_hint.name}</dd>
        </>
      )}
    </dl>
  );
}

function labelForLoader(loader: string): string {
  switch (loader) {
    case "vanilla":
      return "Vanilla";
    case "fabric":
      return "Fabric";
    case "forge":
      return "Forge";
    case "neoforge":
      return "NeoForge";
    case "quilt":
      return "Quilt";
    default:
      return loader;
  }
}

function Spinner() {
  return (
    <span
      className="h-3 w-3 animate-spin rounded-full border-2 border-current border-t-transparent"
      aria-label="처리 중"
    />
  );
}

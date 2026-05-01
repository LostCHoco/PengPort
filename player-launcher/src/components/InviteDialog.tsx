// 초대 링크 (`pengport://join?...`) 클릭 시 표시되는 가입 확인 dialog.
//
// 운영자가 친구에게 보낸 link 를 친구가 클릭하면 OS 가 PengPort 를 launch 하면서
// argv 로 URL 을 전달. lib.rs 의 single_instance + deep_link plugin 이 받아서 main
// window 에 emit. App.tsx 가 URL 을 parse 한 결과를 이 dialog 로 띄움.
//
// 친구의 first-class 동작은 "신뢰하는 운영자가 보낸 link" 라는 가정 위에 있지만, dialog
// 는 url/token 을 명시적으로 표시해서 누가 어디 가입시키는지 사용자가 알 수 있게 한다.
//
// alreadyExists=true 면 같은 URL 이 이미 등록된 상태 — token 갱신만. (link 형식은 동일,
// 처리 분기는 App.tsx 에서.)

import { useEffect, useRef } from "react";
import { Button } from "@/components/ui/button";

export interface InviteRequest {
  url: string;
  token: string;
  /** 같은 URL 이 이미 등록되어 있어 token 만 갱신 (사용자 표시 변경용 플래그). */
  alreadyExists: boolean;
}

interface Props {
  /** null 이면 dialog 닫힘. */
  request: InviteRequest | null;
  onAccept: () => void;
  onDecline: () => void;
  /** 가입 처리 중 (button disable + 라벨 변경). */
  processing?: boolean;
}

export function InviteDialog({ request, onAccept, onDecline, processing = false }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);

  // ESC 닫기 (처리 중에는 무시).
  useEffect(() => {
    if (!request) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !processing) onDecline();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [request, processing, onDecline]);

  // 첫 포커스 — [거부] (의도적으로 보수적 기본값).
  useEffect(() => {
    if (!request) return;
    cardRef.current?.querySelector<HTMLButtonElement>("[data-invite-decline]")?.focus();
  }, [request]);

  if (!request) return null;

  const origin = safeOrigin(request.url) ?? request.url;
  const masked = maskToken(request.token);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onClick={() => {
        if (!processing) onDecline();
      }}
      role="dialog"
      aria-modal="true"
      aria-labelledby="invite-title"
    >
      <div
        ref={cardRef}
        className="w-full max-w-md rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 id="invite-title" className="text-lg font-semibold text-neutral-50">
          {request.alreadyExists ? "토큰 갱신 — 신중" : "PengPort 인스턴스 가입"}
        </h3>

        <div className="mt-3 space-y-3 text-sm text-neutral-300">
          <p>
            {request.alreadyExists ? (
              <>
                <span className="font-mono text-emerald-300">{origin}</span> 의 기존
                토큰을 <span className="font-medium text-red-300">덮어씁니다</span>.
              </>
            ) : (
              <>
                <span className="font-mono text-emerald-300">{origin}</span> 에 가입할까요?
              </>
            )}
          </p>

          <div className="rounded border border-neutral-800 bg-neutral-950/60 p-3 text-xs">
            <div className="text-neutral-500">URL</div>
            <div className="mt-0.5 break-all font-mono text-neutral-200">{request.url}</div>
            <div className="mt-2 text-neutral-500">토큰</div>
            <div className="mt-0.5 font-mono text-neutral-200">{masked}</div>
          </div>

          {request.alreadyExists ? (
            // 보안 경고: 같은 URL 의 token silent overwrite 는 phishing 표적.
            // 운영자가 새 토큰을 명시적으로 보낸 게 아닌데 사용자가 무심코 [갱신] 누르면
            // 공격자 토큰으로 통신하게 됨 → 빨간 경고 박스로 강조.
            <div className="rounded border border-red-900/60 bg-red-950/40 p-3 text-xs text-red-200">
              <p className="font-medium">⚠ 신중히 확인</p>
              <p className="mt-1">
                이 인스턴스는 이미 등록되어 있습니다. 갱신하면 <span className="font-medium">기존 토큰은 사라지고</span>{" "}
                새 토큰으로 통신합니다. 운영자가 직접 "토큰을 갱신했다" 고 안내한 경우만
                진행하세요. 그렇지 않다면 누군가 운영자를 사칭한 phishing 일 수 있습니다.
              </p>
            </div>
          ) : (
            <p className="text-xs text-amber-200/80">
              운영자가 직접 보낸 링크인지 확인하세요. 모르는 출처의 초대 링크는 거부하세요.
            </p>
          )}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <Button
            data-invite-decline
            variant="outline"
            size="sm"
            disabled={processing}
            onClick={onDecline}
            className="cursor-pointer"
          >
            거부
          </Button>
          <Button
            size="sm"
            disabled={processing}
            onClick={onAccept}
            className="min-w-[80px] cursor-pointer"
          >
            {processing
              ? "처리 중..."
              : request.alreadyExists
                ? "갱신"
                : "가입"}
          </Button>
        </div>
      </div>
    </div>
  );
}

function safeOrigin(raw: string): string | null {
  try {
    return new URL(raw).origin;
  } catch {
    return null;
  }
}

function maskToken(token: string): string {
  const t = token.trim();
  if (t.length <= 12) return "•".repeat(Math.max(t.length, 4));
  return `${t.slice(0, 6)}…${t.slice(-4)}`;
}

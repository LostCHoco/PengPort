// 초대 링크 (`pengport://join?...`) 클릭 시 표시되는 가입 확인 dialog.
//
// 운영자가 친구에게 보낸 link 를 친구가 클릭하면 OS 가 PengPort 를 launch 하면서
// argv 로 URL 을 전달. lib.rs 의 single_instance + deep_link plugin 이 받아서 main
// window 에 emit. App.tsx 가 URL 을 parse 한 결과를 이 dialog 로 띄움.
//
// invite B — **토큰은 사용자에게 보이지 않는다**: 링크에는 안정적 `code` 만 있고, 가입을
// 누르면 App.tsx 가 invisibly redeem 해서 현재 토큰을 받아 저장한다. 그래서 이 dialog 는
// 토큰 대신 **운영자/인스턴스 preview**(Tier 1: 이름·운영자)를 fetch 해서 보여준다 —
// 비기술 사용자에게 *누구 서버인지*가 토큰 문자열보다 안전하게 검증된다.
//
// alreadyExists=true 면 같은 URL 이 이미 등록된 상태 — token 갱신만. (link 형식은 동일,
// 처리 분기는 App.tsx 에서.)

import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { pspLoadInstance } from "@/lib/psp/client";
import type { InstanceMetadata } from "@/lib/psp/types";

export interface InviteRequest {
  url: string;
  /** 안정적 초대 코드. 가입 시 redeem 되어 토큰으로 교환됨 (사용자는 토큰을 안 봄). */
  code: string;
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

type PreviewState =
  | { status: "loading" }
  | { status: "ok"; meta: InstanceMetadata }
  | { status: "error"; message: string };

export function InviteDialog({ request, onAccept, onDecline, processing = false }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);
  const [preview, setPreview] = useState<PreviewState>({ status: "loading" });

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

  // Tier 1 preview fetch — "누구 서버인지" 를 토큰 대신 검증. 인스턴스 metadata 는
  // unauthenticated public endpoint 라 토큰/code 없이 조회 가능.
  useEffect(() => {
    if (!request) return;
    let cancelled = false;
    setPreview({ status: "loading" });
    pspLoadInstance(request.url)
      .then((meta) => {
        if (!cancelled) setPreview({ status: "ok", meta });
      })
      .catch((e) => {
        if (!cancelled) setPreview({ status: "error", message: String(e) });
      });
    return () => {
      cancelled = true;
    };
  }, [request]);

  if (!request) return null;

  const origin = safeOrigin(request.url) ?? request.url;

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
          {request.alreadyExists ? "재가입 — 신중" : "PengPort 인스턴스 가입"}
        </h3>

        <div className="mt-3 space-y-3 text-sm text-neutral-300">
          <p>
            {request.alreadyExists
              ? "아래 서버에 다시 가입합니다 (기존 등록을 갱신)."
              : "아래 서버에 가입할까요?"}
          </p>

          {/* Tier 1 운영자/인스턴스 preview — 토큰 대신 "누구 서버인지" 로 검증. */}
          <InstancePreview origin={origin} url={request.url} preview={preview} />

          {request.alreadyExists ? (
            // 보안 경고: 같은 URL 의 재가입(token silent overwrite)은 phishing 표적.
            <div className="rounded border border-red-900/60 bg-red-950/40 p-3 text-xs text-red-200">
              <p className="font-medium">⚠ 신중히 확인</p>
              <p className="mt-1">
                이 인스턴스는 이미 등록되어 있습니다. 진행하면{" "}
                <span className="font-medium">기존 토큰이 새 토큰으로 교체</span>됩니다.
                운영자가 직접 안내한 초대 링크인 경우만 진행하세요. 그렇지 않다면 누군가
                운영자를 사칭한 phishing 일 수 있습니다.
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
            {processing ? "처리 중..." : request.alreadyExists ? "재가입" : "가입"}
          </Button>
        </div>
      </div>
    </div>
  );
}

/** 인스턴스/운영자 preview 박스. 토큰을 대체하는 Tier 1 신뢰 표면. */
function InstancePreview({
  origin,
  url,
  preview,
}: {
  origin: string;
  url: string;
  preview: PreviewState;
}) {
  return (
    <div className="rounded border border-neutral-800 bg-neutral-950/60 p-3 text-xs">
      {preview.status === "loading" && (
        <div className="text-neutral-400">운영자 정보 확인 중…</div>
      )}

      {preview.status === "ok" && (
        <div className="space-y-1.5">
          <div>
            <div className="text-neutral-500">서버</div>
            <div className="mt-0.5 text-sm font-medium text-emerald-300">
              {preview.meta.name}
            </div>
          </div>
          <div>
            <div className="text-neutral-500">운영자</div>
            <div className="mt-0.5 text-neutral-200">
              {preview.meta.operator.name}
              {preview.meta.operator.contact ? (
                <span className="ml-1 text-neutral-500">
                  ({preview.meta.operator.contact})
                </span>
              ) : null}
            </div>
          </div>
          {preview.meta.description ? (
            <div className="text-neutral-400">{preview.meta.description}</div>
          ) : null}
          <div className="mt-1 break-all font-mono text-[11px] text-neutral-500">{url}</div>
        </div>
      )}

      {preview.status === "error" && (
        <div className="space-y-1">
          <div className="text-amber-300">⚠ 운영자 정보를 불러오지 못했습니다.</div>
          <div className="break-all font-mono text-[11px] text-neutral-400">{origin}</div>
          <div className="text-neutral-500">
            서버가 응답하지 않거나 PengPort 인스턴스가 아닐 수 있습니다. 출처가 확실할 때만
            진행하세요.
          </div>
        </div>
      )}
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

// 첫 실행 시 사용 모드 선택 dialog.
//
// 일반 사용자: [내 PC] — 평소 모드. instance / token 영구 저장. 다음 launch 부터 선택 안 받음.
// 공용 PC 사용자 (PC방 등): [공용 PC] — 1회용 모드. 종료 시 모든 데이터 + PengPort 자체 자동 정리.
//
// dialog 자체는 modal — 모드 선택 전 PengPort 사용 차단. backdrop / ESC 로 닫기 X (모드 미선택 상태가
// 잘못된 default — 평소 모드든 1회용 모드든 명시 선택 강제).

import { useRef } from "react";
import type { Mode } from "@/lib/mode";

interface Props {
  /** 사용자가 모드 선택 시 호출. */
  onSelect: (mode: Mode) => void;
}

export function ModeSelectorDialog({ onSelect }: Props) {
  const cardRef = useRef<HTMLDivElement>(null);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-4"
      role="dialog"
      aria-modal="true"
      aria-labelledby="mode-selector-title"
    >
      <div
        ref={cardRef}
        className="w-full max-w-lg rounded-lg border border-neutral-800 bg-neutral-900 p-6 shadow-2xl"
      >
        <h3
          id="mode-selector-title"
          className="text-lg font-semibold text-neutral-50"
        >
          PengPort 환영합니다
        </h3>

        <p className="mt-2 text-sm text-neutral-400">
          이 PC 에서 PengPort 를 어떻게 사용하시나요?
        </p>

        <div className="mt-5 grid grid-cols-1 gap-3">
          <ModeOption
            title="내 PC (평소 사용)"
            desc="인스턴스 / 토큰 / 게임 설정을 영구 저장합니다. 평소 자기 PC 에서 사용하는 일반 모드."
            cta="이 PC 에서 평소 사용"
            onClick={() => onSelect("normal")}
            primary
          />
          <ModeOption
            title="공용 PC (1회용 모드)"
            desc="PC방 / 친구 PC 등 일시 사용. PengPort 종료 시 모든 데이터 (인스턴스 / 토큰 / Prism 계정 / Minecraft 세이브) 와 PengPort 자체가 자동 정리됩니다. 흔적 0."
            cta="1회용 모드로 시작"
            onClick={() => onSelect("ephemeral")}
            warn
          />
        </div>

        <p className="mt-4 text-xs text-neutral-500">
          잘못 선택해도 Settings 에서 변경 가능합니다. 1회용 모드는 종료 시 자동 정리 + 자동 제거됩니다.
        </p>
      </div>
    </div>
  );
}

interface OptionProps {
  title: string;
  desc: string;
  cta: string;
  onClick: () => void;
  primary?: boolean;
  warn?: boolean;
}

function ModeOption({ title, desc, cta, onClick, primary, warn }: OptionProps) {
  const border = primary
    ? "border-emerald-700/60 hover:border-emerald-500"
    : warn
      ? "border-amber-700/60 hover:border-amber-500"
      : "border-neutral-700 hover:border-neutral-500";

  return (
    <button
      type="button"
      onClick={onClick}
      className={`w-full rounded-md border ${border} bg-neutral-950/40 p-4 text-left transition-colors hover:bg-neutral-950/70 focus:outline-none focus:ring-2 focus:ring-emerald-500/40 cursor-pointer`}
    >
      <div
        className={
          primary
            ? "text-sm font-medium text-emerald-300"
            : warn
              ? "text-sm font-medium text-amber-300"
              : "text-sm font-medium text-neutral-200"
        }
      >
        {title}
      </div>
      <p className="mt-1 text-xs text-neutral-400">{desc}</p>
      <div
        className={
          primary
            ? "mt-2 text-xs text-emerald-400"
            : warn
              ? "mt-2 text-xs text-amber-400"
              : "mt-2 text-xs text-neutral-300"
        }
      >
        → {cta}
      </div>
    </button>
  );
}

// 사용 모드 (normal / ephemeral) React context.
//
// App.tsx 의 헤더 배지, ModeSelectorDialog, EphemeralExitDialog (close hook), Settings 의
// 모드 변경 UI 가 모두 같은 source 를 공유. localStorage 가 source of truth, React state 는 mirror.
//
// `setMode` 동작:
//   normal     → localStorage 에 저장 (다음 launch 부터 selector 안 묻기)
//   ephemeral  → localStorage 저장 안 함 (다음 launch 시 또 selector 묻기). React state 만 변경.
//   null       → localStorage 의 mode 키 제거 (다음 launch 시 selector). 모드 reset 용.

import {
  createContext,
  type ReactNode,
  useCallback,
  useContext,
  useState,
} from "react";
import { getMode, setMode as persistMode, type Mode } from "./mode";

interface ModeContextValue {
  mode: Mode | null;
  /** 모드 변경. null 전달 시 localStorage 에서 키 제거 (다음 launch 시 selector). */
  setMode: (next: Mode | null) => void;
}

const ModeContext = createContext<ModeContextValue | null>(null);

export function ModeProvider({ children }: { children: ReactNode }) {
  const [mode, setModeState] = useState<Mode | null>(() => getMode());

  const setMode = useCallback((next: Mode | null) => {
    if (next === null) {
      localStorage.removeItem("pengport.mode");
    } else {
      persistMode(next); // normal 만 저장됨 (mode.ts 의 정책)
    }
    setModeState(next);
  }, []);

  return (
    <ModeContext.Provider value={{ mode, setMode }}>
      {children}
    </ModeContext.Provider>
  );
}

export function useMode(): ModeContextValue {
  const ctx = useContext(ModeContext);
  if (!ctx) throw new Error("useMode must be used inside <ModeProvider>");
  return ctx;
}

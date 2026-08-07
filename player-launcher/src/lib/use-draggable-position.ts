// 다이얼로그 헤더를 드래그해 카드를 화면 안에서 옮길 수 있게 하는 공용 훅. 이
// 프로젝트의 모든 modal(`bg-black/60` 백드롭 패턴)이 손수 만든 컴포넌트라 shadcn
// Dialog 같은 공용 프리미티브가 없고, 드래그 이동도 각자 만들면 중복이라 여기 하나로
// 모은다.
//
// `active`가 false→true로 바뀔 때(다이얼로그가 새로 열릴 때)마다 오프셋을 {0,0}으로
// 리셋 — 항상 화면 중앙에서 다시 시작한다. 부모가 언마운트/재마운트되는 다이얼로그는
// 그 자체로 매번 리셋되므로 `active`에 항상 `true`를 넘겨도 무방.

import { useEffect, useRef, useState } from "react";

interface DragState {
  startX: number;
  startY: number;
  baseX: number;
  baseY: number;
}

export function useDraggablePosition(active: boolean) {
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const dragRef = useRef<DragState | null>(null);

  useEffect(() => {
    if (active) setOffset({ x: 0, y: 0 });
  }, [active]);

  const onHeaderMouseDown = (e: React.MouseEvent) => {
    if (e.button !== 0) return;
    dragRef.current = { startX: e.clientX, startY: e.clientY, baseX: offset.x, baseY: offset.y };
    const onMove = (ev: MouseEvent) => {
      const drag = dragRef.current;
      if (!drag) return;
      setOffset({ x: drag.baseX + (ev.clientX - drag.startX), y: drag.baseY + (ev.clientY - drag.startY) });
    };
    const onUp = () => {
      dragRef.current = null;
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  return {
    style: { transform: `translate(${offset.x}px, ${offset.y}px)` },
    onHeaderMouseDown,
  };
}

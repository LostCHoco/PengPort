import { type ReactNode } from "react";
import { createPortal } from "react-dom";

/** `children`을 `document.body`에 직접 붙인다 — 조상 중 하나가 `isolate`(또는
 * `transform`/`filter` 등으로 새 스태킹 컨텍스트를 만드는 속성)를 걸어도 그 안에
 * 갇히지 않는다. 전체 화면 모달이나 트리거를 따라다니는 팝오버처럼, 조상의 레이아웃/
 * 스태킹 컨텍스트와 무관하게 항상 맨 위에 떠야 하는 것에 쓴다. */
export function Portal({ children }: { children: ReactNode }) {
  return createPortal(children, document.body);
}

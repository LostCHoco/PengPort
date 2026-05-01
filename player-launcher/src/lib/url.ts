/**
 * 두 URL 이 같은 origin (scheme + host + port) 인지 검사.
 *
 * PSP 의 same-origin policy 적용에 사용 — catalog/manifest URL 이 instance origin
 * 안에 있는지 검증하기 위함. token 누출 차단.
 *
 * 둘 중 하나라도 parse 실패하면 false (보수적 — 잘 모르는 형식은 거부).
 */
export function isSameOrigin(a: string, b: string): boolean {
  try {
    return new URL(a).origin === new URL(b).origin;
  } catch {
    return false;
  }
}

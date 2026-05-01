// 초대 링크 (`pengport://join?url=...&token=...`) 빌더.
//
// 운영자가 Settings 의 [초대 링크 복사] 클릭 시 사용. 친구 측 parsing 은 App.tsx 의
// parseInviteUrl 에 있다 (private — 같은 형식이지만 검증 규칙이 다르므로 분리 유지).
//
// percent-encoding 은 URLSearchParams 가 자동 처리. token 에 `&`, `=` 등이 들어가도 안전.

export function buildInviteUrl(input: { url: string; token: string }): string {
  const params = new URLSearchParams();
  params.set("url", input.url);
  params.set("token", input.token);
  return `pengport://join?${params.toString()}`;
}

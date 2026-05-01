// 초대 링크 빌더.
//
// 두 형식 — 같은 destination 의 두 표현:
//
// 1. **HTTPS landing** (`buildInviteLandingUrl`): `<instance.url>/invite?token=...`
//    디스코드/카톡 등 메시지 앱에서 자동 hyperlink. 친구가 클릭 → 브라우저 → gateway 의
//    `/invite` HTML 페이지 → meta refresh → `pengport://join?...` → PengPort 의
//    InviteDialog. **운영자가 친구에게 보낼 때 사용.**
//
// 2. **Deep link 직접** (`buildInviteDeepLink`): `pengport://join?url=...&token=...`
//    custom scheme 이라 메시지 앱에서 hyperlink 안 됨. PowerShell `Start-Process` 같은
//    직접 호출 / 디버깅 / 특수 케이스용.
//
// 친구 측 parsing 은 App.tsx 의 parseInviteUrl 에 있다 (`pengport://join?...` 만 받음).
// HTTPS landing 은 브라우저가 처리해서 deep link 로 redirect → 같은 parser 도달.

/** HTTPS landing URL — 메시지 앱에서 자동 hyperlink. 운영자가 친구에게 보낼 표준 형식. */
export function buildInviteLandingUrl(input: { url: string; token: string }): string {
  const base = input.url.replace(/\/+$/, "");
  const params = new URLSearchParams();
  params.set("token", input.token);
  return `${base}/invite?${params.toString()}`;
}

/** Deep link 직접 형식 — custom scheme. 디버깅 / PowerShell 직접 호출 용도. */
export function buildInviteDeepLink(input: { url: string; token: string }): string {
  const params = new URLSearchParams();
  params.set("url", input.url);
  params.set("token", input.token);
  return `pengport://join?${params.toString()}`;
}

/** @deprecated 이전 이름. `buildInviteDeepLink` 사용. */
export const buildInviteUrl = buildInviteDeepLink;

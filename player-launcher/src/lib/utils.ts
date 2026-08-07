import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

/** 사람이 읽는 바이트 크기 표시(다운로드 진행률, base64 콘텐츠 요약 등 공용). */
export function formatBytes(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}GB`
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(0)}MB`
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}KB`
  return `${n}B`
}

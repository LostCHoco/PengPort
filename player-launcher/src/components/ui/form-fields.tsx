// 편집 다이얼로그(`RecipeEditDialog`, `ThirdPartyAppEditDialog`) 공용 입력 프리미티브.
// 원래 `RecipeEditDialog.tsx` 안에만 있던 것을 두 번째 사용처가 생기면서 분리.

export const inputClass =
  "w-full rounded border border-neutral-800 bg-neutral-950 px-2 py-1.5 text-sm text-neutral-200 outline-none focus:border-neutral-600 disabled:opacity-50";

export function Field({
  label,
  action,
  className,
  children,
}: {
  label: string;
  /** 라벨 오른쪽에 붙는 버튼 등(예: "+ 추가"). */
  action?: React.ReactNode;
  /** 부모가 flex 컨테이너일 때 크기 제어용(예: `shrink-0`). */
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div className={className}>
      <div className="mb-1 flex items-center justify-between gap-2">
        <label className="text-xs font-medium text-neutral-400">{label}</label>
        {action}
      </div>
      {children}
    </div>
  );
}

export function TextInput({
  value,
  onChange,
  placeholder,
  disabled,
  readOnly,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  disabled?: boolean;
  /** 타이핑은 막고 값 확인·선택·복사는 그대로 허용 — "찾아보기" 피커로만 값을
   * 채우게 하고 싶을 때(오타 방지) `disabled` 대신 이걸 쓴다. */
  readOnly?: boolean;
}) {
  return (
    <input
      type="text"
      className={`${inputClass} ${readOnly ? "cursor-default bg-neutral-900/60" : ""}`}
      value={value}
      placeholder={placeholder}
      disabled={disabled}
      readOnly={readOnly}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}

export function Select({
  value,
  onChange,
  options,
}: {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  return (
    <select className={inputClass} value={value} onChange={(e) => onChange(e.target.value)}>
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

export function RemoveButton({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="shrink-0 cursor-pointer rounded px-1.5 py-1 text-xs text-red-300 hover:bg-red-950/40"
      aria-label="삭제"
    >
      ✕
    </button>
  );
}

/** `readOnly` + "찾아보기" 피커로만 채워지는 경로 필드(예: `extract_to`,
 * `path_overrides.to`, `entry_point`) 옆에 두는 값 초기화 버튼 — 피커는 트리에
 * 이미 선언된 값만 고를 수 있어, 값이 잘못 꼬였을 때 되돌릴 방법이 이것뿐이다.
 * 값이 비어있으면 지울 게 없으니 렌더링 자체를 생략. */
export function ClearFieldButton({ value, onClear }: { value: string; onClear: () => void }) {
  if (!value) return null;
  return (
    <button
      type="button"
      onClick={onClear}
      className="shrink-0 cursor-pointer rounded px-1.5 py-1 text-xs text-neutral-400 hover:bg-neutral-800 hover:text-neutral-200"
      aria-label="지우기"
      title="지우기"
    >
      지우기
    </button>
  );
}

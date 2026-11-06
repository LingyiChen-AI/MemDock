import * as React from "react"
import { MinusIcon, PlusIcon } from "lucide-react"
import { cn } from "@/lib/utils"

/**
 * Integer field with -/+ steppers instead of the native spinner. The text stays a string
 * so the caller can validate (and show) half-typed values; the steppers clamp to [min, max].
 */
function NumberInput({
  value,
  onChange,
  min,
  max,
  step = 1,
  className,
  disabled,
  decrementLabel = "-",
  incrementLabel = "+",
  ...props
}: Omit<React.ComponentProps<"input">, "value" | "onChange" | "type" | "min" | "max" | "step"> & {
  value: string
  onChange: (value: string) => void
  min?: number
  max?: number
  step?: number
  decrementLabel?: string
  incrementLabel?: string
}) {
  const n = Number(value)
  const clamp = (v: number) => Math.min(max ?? Infinity, Math.max(min ?? -Infinity, v))
  const bump = (dir: 1 | -1) => {
    const base = value.trim() === "" || !Number.isFinite(n) ? (min ?? 0) : n
    onChange(String(clamp(base + dir * step)))
  }
  const btn =
    "grid h-full w-7 shrink-0 cursor-pointer place-items-center text-muted-foreground transition-colors outline-none hover:bg-muted hover:text-foreground focus-visible:bg-muted disabled:cursor-not-allowed disabled:opacity-40 motion-reduce:transition-none"
  return (
    <div
      data-slot="number-input"
      aria-invalid={props["aria-invalid"]}
      className={cn(
        "inline-flex h-8 items-stretch overflow-hidden rounded-md border border-input bg-transparent transition-colors focus-within:border-ring focus-within:ring-2 focus-within:ring-ring aria-invalid:border-destructive aria-invalid:ring-2 aria-invalid:ring-destructive/20 dark:bg-input/30",
        disabled && "opacity-50",
        className
      )}
    >
      <button
        type="button"
        tabIndex={-1}
        aria-label={decrementLabel}
        disabled={disabled || (min !== undefined && Number.isFinite(n) && value.trim() !== "" && n <= min)}
        onClick={() => bump(-1)}
        className={cn(btn, "border-r border-input")}
      >
        <MinusIcon className="size-3.5" aria-hidden="true" />
      </button>
      <input
        type="text"
        inputMode="numeric"
        data-slot="input"
        role="spinbutton"
        aria-valuemin={min}
        aria-valuemax={max}
        aria-valuenow={Number.isFinite(n) && value.trim() !== "" ? n : undefined}
        value={value}
        disabled={disabled}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "ArrowUp") {
            e.preventDefault()
            bump(1)
          } else if (e.key === "ArrowDown") {
            e.preventDefault()
            bump(-1)
          }
        }}
        className="w-full min-w-0 bg-transparent px-1 text-center text-sm tabular-nums outline-none disabled:cursor-not-allowed"
        {...props}
      />
      <button
        type="button"
        tabIndex={-1}
        aria-label={incrementLabel}
        disabled={disabled || (max !== undefined && Number.isFinite(n) && value.trim() !== "" && n >= max)}
        onClick={() => bump(1)}
        className={cn(btn, "border-l border-input")}
      >
        <PlusIcon className="size-3.5" aria-hidden="true" />
      </button>
    </div>
  )
}

export { NumberInput }

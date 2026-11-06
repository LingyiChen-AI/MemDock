import * as React from "react"
import { CalendarIcon } from "lucide-react"
import { cn } from "@/lib/utils"
import { Calendar } from "@/components/ui/calendar"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"

const pad = (n: number) => String(n).padStart(2, "0")

/** "YYYY-MM-DD" (local) -> Date, or undefined when malformed. */
export function parseYmd(s: string | undefined): Date | undefined {
  const m = s ? /^(\d{4})-(\d{2})-(\d{2})$/.exec(s) : null
  if (!m) return undefined
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]))
}

/** Date -> "YYYY-MM-DD" in local time. */
export function formatYmd(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

/**
 * Button + popover calendar replacing `<input type="date">`.
 * `value`, `min` and `max` are "YYYY-MM-DD" strings, like the native input.
 */
function DatePicker({
  value,
  onChange,
  min,
  max,
  placeholder,
  disabled,
  id,
  className,
  "aria-label": ariaLabel,
}: {
  value: string
  onChange: (value: string) => void
  min?: string
  max?: string
  placeholder?: string
  disabled?: boolean
  id?: string
  className?: string
  "aria-label"?: string
}) {
  const [open, setOpen] = React.useState(false)
  const selected = parseYmd(value)
  const minDate = parseYmd(min)
  const maxDate = parseYmd(max)
  const disabledDays = [
    ...(minDate ? [{ before: minDate }] : []),
    ...(maxDate ? [{ after: maxDate }] : []),
  ]
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          id={id}
          aria-label={ariaLabel}
          disabled={disabled}
          className={cn(
            "inline-flex h-8 cursor-pointer items-center gap-2 rounded-md border border-input bg-transparent px-2.5 text-sm whitespace-nowrap tabular-nums transition-colors outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 aria-expanded:border-ring dark:bg-input/30 dark:hover:bg-input/50",
            !selected && "text-muted-foreground",
            className
          )}
        >
          <CalendarIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span>{selected ? value : placeholder}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-auto p-2">
        <Calendar
          mode="single"
          selected={selected}
          defaultMonth={selected}
          disabled={disabledDays}
          startMonth={minDate}
          endMonth={maxDate}
          onSelect={(d) => {
            if (!d) return
            onChange(formatYmd(d))
            setOpen(false)
          }}
          autoFocus
        />
      </PopoverContent>
    </Popover>
  )
}

export { DatePicker }

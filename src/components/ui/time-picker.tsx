import * as React from "react"
import { ClockIcon } from "lucide-react"
import { cn } from "@/lib/utils"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"

const pad = (n: number) => String(n).padStart(2, "0")
const HOURS = Array.from({ length: 24 }, (_, i) => i)

/**
 * "HH:MM" picker replacing `<input type="time">`: a trigger plus hour / minute columns.
 * `minuteStep` controls the minute column (default every 5 minutes, the current value is always listed).
 */
function TimePicker({
  value,
  onChange,
  minuteStep = 5,
  hourLabel = "Hour",
  minuteLabel = "Minute",
  disabled,
  id,
  className,
  "aria-label": ariaLabel,
}: {
  value: string
  onChange: (value: string) => void
  minuteStep?: number
  hourLabel?: string
  minuteLabel?: string
  disabled?: boolean
  id?: string
  className?: string
  "aria-label"?: string
}) {
  const m = /^(\d{1,2}):(\d{2})$/.exec(value)
  const hh = m ? Number(m[1]) : 0
  const mm = m ? Number(m[2]) : 0
  const minutes = React.useMemo(() => {
    const list = Array.from({ length: Math.ceil(60 / minuteStep) }, (_, i) => i * minuteStep)
    if (!list.includes(mm)) list.push(mm)
    return list.sort((a, b) => a - b)
  }, [minuteStep, mm])

  const column = (label: string, items: number[], current: number, pick: (n: number) => void) => (
    <div role="listbox" aria-label={label} className="flex h-52 w-14 flex-col gap-0.5 overflow-y-auto p-1">
      {items.map((n) => (
        <button
          key={n}
          type="button"
          role="option"
          aria-selected={n === current}
          ref={(el) => {
            if (el && n === current) el.scrollIntoView?.({ block: "center" })
          }}
          onClick={() => pick(n)}
          className={cn(
            "shrink-0 cursor-pointer rounded-sm py-1 text-center text-sm tabular-nums outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring",
            n === current && "bg-primary text-primary-foreground hover:bg-primary/80"
          )}
        >
          {pad(n)}
        </button>
      ))}
    </div>
  )

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          id={id}
          aria-label={ariaLabel}
          disabled={disabled}
          className={cn(
            "inline-flex h-8 cursor-pointer items-center gap-2 rounded-md border border-input bg-transparent px-2.5 text-sm tabular-nums transition-colors outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 aria-expanded:border-ring dark:bg-input/30 dark:hover:bg-input/50",
            className
          )}
        >
          <ClockIcon className="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
          <span>{`${pad(hh)}:${pad(mm)}`}</span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="flex w-auto divide-x divide-border p-0">
        {column(hourLabel, HOURS, hh, (h) => onChange(`${pad(h)}:${pad(mm)}`))}
        {column(minuteLabel, minutes, mm, (n) => onChange(`${pad(hh)}:${pad(n)}`))}
      </PopoverContent>
    </Popover>
  )
}

export { TimePicker }

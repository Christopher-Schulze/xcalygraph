import { forwardRef, memo } from "react"

import { useTheme } from "../context/theme"
import { useCounter } from "../hooks/use-counter"

export const Button = forwardRef<HTMLButtonElement, { label: string }>(({ label }, ref) => {
  const theme = useTheme()
  const { count, increment } = useCounter(0)
  return (
    <button ref={ref} className={theme} onClick={increment}>
      {label} {count}
    </button>
  )
})

export const Badge = memo(function Badge({ text }: { text: string }) {
  return <Button label={text} />
})

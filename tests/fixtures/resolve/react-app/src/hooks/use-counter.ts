import { useState } from "react"

export function useCounter(start: number) {
  const [count, setCount] = useState(start)
  return { count, increment: () => setCount(count + 1) }
}

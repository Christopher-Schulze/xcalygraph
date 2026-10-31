import { createContext, useContext, useState } from "react"

const ThemeContext = createContext("light")

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [theme] = useState("light")
  return <ThemeContext.Provider value={theme}>{children}</ThemeContext.Provider>
}

export function useTheme() {
  return useContext(ThemeContext)
}

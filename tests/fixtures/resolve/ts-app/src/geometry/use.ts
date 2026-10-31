import { grow, Geometry } from "./index"

// Object-literal methods resolve through the object.
const config = {
  total(): number {
    return grow(2) + Geometry.unit()
  },
}

export function main(): number {
  return config.total()
}

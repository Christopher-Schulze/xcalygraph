// Overload signatures declare the implementation; namespace members resolve through the
// namespace.
export function scale(x: number): number
export function scale(x: number, by: number): number
export function scale(x: number, by = 2): number {
  return x * by
}

export namespace Geometry {
  export function unit(): number {
    return 1
  }
}

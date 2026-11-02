import type { Private } from "./provider";
export function usePrivate(value: Private): number { return value.run(); }

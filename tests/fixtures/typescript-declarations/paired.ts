export function make(id: string): string { return id; }
export function normalize(value: string): string;
export function normalize(value: number): string;
export function normalize(value: string | number): string { return String(value); }

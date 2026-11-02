interface Left { id: string; }
interface Right { id: number; }
interface Conflict extends Left, Right { a: string; }
interface Conflict extends Left, Right { b: number; }
interface LoopA extends LoopB { a: string; }
interface LoopB extends LoopA { b: string; }
interface Cycle extends LoopA { a: string; }
interface Cycle extends LoopA { b: string; }
export function inspect(value: Conflict): string { return value.id; }
interface OverBase { parse(value: string): string; parse(value: number): number; }
interface Incomplete extends OverBase { parse(value: string): string; }
interface Incomplete extends OverBase { extra: boolean; }
interface DifferentConflict extends Left { first: string; }
interface DifferentConflict extends Right { second: number; }

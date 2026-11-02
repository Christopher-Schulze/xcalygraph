interface Root { id: string; service: Service; root(): number; }
interface Left extends Root { left(): number; }
interface Right { right: boolean; }
interface Child extends Left, Right { a: string; }
interface Child extends Left, Right { b: number; }
export class Service { execute(): number { return 1; } }
export function inspect(value: Child): string { return value.id; }
export function callRoot(value: Child): number { return value.root(); }
export function callService(value: Child): number { return value.service.execute(); }
interface OverBase { parse(value: string): string; parse(value: number): number; }
interface OverChild extends OverBase { parse(value: string): string; }
interface OverChild extends OverBase { parse(value: number): number; extra: boolean; }
interface Different extends Right { first: string; }
interface Different extends Root { second: number; }
export function callDifferent(value: Different): number { return value.root(); }

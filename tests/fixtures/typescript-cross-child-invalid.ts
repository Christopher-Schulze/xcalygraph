import type { Base } from "./base";
interface Child extends Base { extra: string; }
interface Child extends Base { right: number; }
export function inspect(value: Child): string { return value.extra; }

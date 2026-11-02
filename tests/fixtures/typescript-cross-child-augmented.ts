import type { Base } from "./base";
interface Child extends Base { left: number; }
interface Child extends Base { right: number; }
export function callExtra(value: Child): number { return value.extra(); }

import type { Base } from "./base";
interface Child extends Base { left: number; }
interface Child extends Base { right: number; }
export function call(value: Child): number { return value.run(); }
export interface Public extends Base { first: boolean; }
export interface Public extends Base { second: boolean; }

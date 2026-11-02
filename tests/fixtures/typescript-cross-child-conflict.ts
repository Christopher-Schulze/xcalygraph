import type { Base } from "./base";
interface Child extends Base { id: number; }
interface Child extends Base { right: number; }
export function inspect(value: Child): number { return value.id; }

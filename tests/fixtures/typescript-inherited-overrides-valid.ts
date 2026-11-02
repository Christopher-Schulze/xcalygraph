interface Base { readonly id: string; }
interface Child extends Base { readonly "id": string; left: number; }
interface Child extends Base { right: number; }
export function inspect(value: Child): string { return value.id; }
interface MethodBase { run(): number; }
interface MethodChild extends MethodBase { run(): number; }
interface MethodChild extends MethodBase { right: number; }

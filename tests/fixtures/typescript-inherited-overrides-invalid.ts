interface Base { readonly id: string; }
interface Bad extends Base { id: number; }
interface Bad extends Base { right: number; }
export function inspect(value: Bad): number { return value.id; }

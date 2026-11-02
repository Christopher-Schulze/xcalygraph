namespace Other { export class Hidden { run(): number { return 1; } } }
export function outside(value: Hidden): number { return value.run(); }

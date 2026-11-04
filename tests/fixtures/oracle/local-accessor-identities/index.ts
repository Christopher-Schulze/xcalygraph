export function first() {}
export function outer() { return {get value() { first(); return 1; }}; }

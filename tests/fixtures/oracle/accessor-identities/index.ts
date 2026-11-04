export function first() {}
export class Box {
    get value() { first(); return 1; }
}

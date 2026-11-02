interface Box<T> { item: T; }
interface Child extends Box<Service> { left: string; }
interface Child extends Box<Service> { right: number; }
interface Override extends Box<Service> { item: Service; }
interface Override extends Box<Service> { right: number; }
class T { execute(): number { return 2; } toUpperCase(): string { return "wrong"; } }
export class Service { execute(): number { return 1; } }
export function call(value: Child): number { return value.item.execute(); }
interface Pair<A, B> { first: A; second: B; }
interface PairChild extends Pair<Service, Other> { left: string; }
interface PairChild extends Pair<Service, Other> { right: number; }
class Other { other(): number { return 3; } }
export function second(value: PairChild): number { return value.second.other(); }
interface Text extends Box<string> { left: string; }
interface Text extends Box<string> { right: number; }
export function upper(value: Text): string { return value.item.toUpperCase(); }
interface List<T> { items: T[]; }
interface Nested extends List<Service> { left: string; }
interface Nested extends List<Service> { right: number; }
interface NestedOverride extends List<Service> { items: Service[]; }
interface NestedOverride extends List<Service> { right: number; }

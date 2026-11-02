export class Service { execute(): number { return 1; } }
interface Pair<A, B = A> { first: A; second: B; }
interface Child extends Pair<Service> { left: string; }
interface Child extends Pair<Service> { right: number; }
export function call(value: Child): number { return value.second.execute(); }
interface Concrete<A, B = Service> { first: A; second: B; }
interface Explicit extends Concrete<string, Service> { left: string; }
interface Explicit extends Concrete<string, Service> { right: number; }
interface Defaulted extends Concrete<string> { left: string; }
interface Defaulted extends Concrete<string> { right: number; }
export function callDefault(value: Defaulted): number { return value.second.execute(); }
interface DefaultBox<T = Service> { item: T; }
interface BareDefault extends DefaultBox { left: string; }
interface BareDefault extends DefaultBox { right: number; }
export function callBare(value: BareDefault): number { return value.item.execute(); }

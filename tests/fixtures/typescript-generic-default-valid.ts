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
interface Root<T> { value: T; }
interface Middle<T> extends Root<T> { middle: string; }
interface Transitive extends Middle<Service> { left: string; }
interface Transitive extends Middle<Service> { right: number; }
export function callTransitive(value: Transitive): number { return value.value.execute(); }
interface Left<T> { leftValue: T; }
interface Right<T> { rightValue: T; }
interface Multiple extends Left<Service>, Right<Service> { left: string; }
interface Multiple extends Left<Service>, Right<Service> { right: number; }
export function callMultiple(value: Multiple): number { return value.rightValue.execute(); }
interface Shared<T> { shared: T; }
interface First<T> extends Shared<T> { first: string; }
interface Second<T> extends Shared<T> { second: string; }
interface Diamond extends First<Service>, Second<Service> { left: string; }
interface Diamond extends First<Service>, Second<Service> { right: number; }
export function callDiamond(value: Diamond): number { return value.shared.execute(); }
interface Constrained<T extends Service> { item: T; }
interface ConstraintChild extends Constrained<Service> { left: string; }
interface ConstraintChild extends Constrained<Service> { right: number; }
export function callConstrained(value: ConstraintChild): number { return value.item.execute(); }
interface Factory<T> { make(): T; }
interface MethodChild extends Factory<Service> { left: string; }
interface MethodChild extends Factory<Service> { right: number; }

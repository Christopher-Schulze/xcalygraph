export class Service { execute(): number { return 1; } }
interface Pair<A, B = A> { second: B; }
interface Conflict extends Pair<Service> { second: string; }
interface Conflict extends Pair<Service> { right: number; }
interface Missing extends Pair { left: string; }
interface Missing extends Pair { right: number; }
interface Constrained<A, B extends A = A> { second: B; }
interface Unproven extends Constrained<Service, string> { left: string; }
interface Unproven extends Constrained<Service, string> { right: number; }
interface Circular<A = A> { item: A; }
interface BadDefault extends Circular<Service> { left: string; }
interface BadDefault extends Circular<Service> { right: number; }
interface Root<T> { value: T; }
interface Middle<T> extends Root<T> { middle: string; }
interface BadTransitive extends Middle<Service> { value: number; }
interface BadTransitive extends Middle<Service> { right: number; }
interface Left<T> { value: T; }
interface Right<T> { value: T; }
interface BadMultiple extends Left<Service>, Right<string> { left: string; }
interface BadMultiple extends Left<Service>, Right<string> { right: number; }
interface CycleA<T> extends CycleB<T> { a: T; }
interface CycleB<T> extends CycleA<T> { b: T; }
interface Cyclic extends CycleA<Service> { left: string; }
interface Cyclic extends CycleA<Service> { right: number; }
interface Restricted<T extends Service = string> { item: T; }
interface BadConstrainedDefault extends Restricted<Service> { left: string; }
interface BadConstrainedDefault extends Restricted<Service> { right: number; }
interface Cross<A, B extends Service = A> { item: B; }
interface BadDependentDefault extends Cross<Service> { left: string; }
interface BadDependentDefault extends Cross<Service> { right: number; }
interface Bounded<T extends Service> { item: T; }
interface Unbounded<T> extends Bounded<T> { middle: string; }
interface BadTransitiveConstraint extends Unbounded<Service> { left: string; }
interface BadTransitiveConstraint extends Unbounded<Service> { right: number; }
interface OverrideRoot<T> { item: T; }
interface BadOverride<T> extends OverrideRoot<T> { item: Service; }
interface BadGenericOverride extends BadOverride<Service> { left: string; }
interface BadGenericOverride extends BadOverride<Service> { right: number; }

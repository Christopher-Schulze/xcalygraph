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

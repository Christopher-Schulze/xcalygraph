interface Box<T> { item: T; }
interface Conflict extends Box<string> { item: number; }
interface Conflict extends Box<string> { right: number; }
interface First extends Box<string> { left: string; }
interface First extends Box<number> { right: number; }
export function use(value: Conflict): number { return value.item; }
interface Constrained<T extends { execute(): number }> { item: T; }
interface BadConstraint extends Constrained<string> { left: string; }
interface BadConstraint extends Constrained<string> { right: number; }
interface Qualified<T> { item: T.Member; }
interface BadQualified extends Qualified<string> { left: string; }
interface BadQualified extends Qualified<string> { right: number; }
interface Ren\u0061med<T> { item: number; }
interface Renamed<T> { item: T; }
interface EscapedChild extends Renamed<string> { left: string; }
interface EscapedChild extends Renamed<string> { right: number; }
interface Norm\u0061lBase { id: number; }
interface NormalBase { id: string; }
interface NonGenericChild extends NormalBase { left: string; }
interface NonGenericChild extends NormalBase { right: number; }
interface N\u0061med { value: number; }
interface Named { value: string; }
interface Named { other: number; }

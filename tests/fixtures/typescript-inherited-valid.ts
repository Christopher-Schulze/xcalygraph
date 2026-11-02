interface Base { id: string; run(): number; service: Service; }
interface Inherited extends Base { left: number; }
interface Inherited extends Base { right: number; }
export function inspect(value: Inherited): string { return value.id; }
export function callBase(value: Inherited): number { return value.run(); }
export interface GenericBase<T> { value: T; }
export interface GenericChild extends GenericBase<string> { left: number; }
export interface GenericChild extends GenericBase<string> { right: number; }
export class Service { execute(): number { return 1; } }
export function callField(value: Inherited): number { return value.service.execute(); }
export class OtherService { execute(): number { return 2; } }

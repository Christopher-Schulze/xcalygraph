export class Service { run(): number { return 1; } }
export function outside(value: Service): number { return value.run(); }
export function generic<Service extends { run(): number }>(value: Service): number { return value.run(); }
export function genericArrow<T extends () => void, Service extends { run(): number }>(value: Service): number { return value.run(); }
export const arrow = <Service extends { run(): number }>(value: Service): number => value.run();
export const asyncArrow = async<Service extends { run(): number }>(value: Service) => value.run();
interface GenericBox<Service extends { run(): number }> { item: Service; }
export function genericField<Service extends { run(): number }>(value: GenericBox<Service>): number { return value.item.run(); }
export namespace Area {
    export class Service { run(): number { return 2; } }
    export function inside(value: Service): number { return value.run(); }
}

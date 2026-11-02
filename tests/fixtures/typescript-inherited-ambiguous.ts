interface Base { service: Service; }
interface Derived extends Base { left: number; }
interface Derived extends Base { right: number; }
namespace Other { export interface Derived { service: OtherService; } }
export class Service { execute(): number { return 1; } }
export class OtherService { execute(): number { return 2; } }
export function run(value: Derived): number { return value.service.execute(); }

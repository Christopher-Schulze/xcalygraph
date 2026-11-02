import type { Imported } from "./provider";
namespace Other { export class Imported { run(): number { return 3; } } }
export function viaImport(value: Imported): number { return value.run(); }

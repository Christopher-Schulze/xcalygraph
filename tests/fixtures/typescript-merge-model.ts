export interface Config<T> { width: number; service: T; run(value: string): string; }
export interface Config<T> { height: number; extraService: T; run(value: number): number; }
export namespace Nested { export interface Config<T> { nested: T; } }
export function area(config: Config<string>): number { return config.width * config.height; }
export class Service { run(): number { return 1; } }
export class OtherService { run(): number { return 2; } }
export interface Services { first: Service; }
export interface Services { extra: Service; }
export function useService(services: Services): number { return services.extra.run(); }
export interface Worker { start(): void; }
export interface Worker { finish(): void; }
export function useWorker(worker: Worker): void { worker.finish(); }
export interface Query { execute(value: string): number; }
export interface Query { execute(value: string, limit: number): number; }
export function useQuery(query: Query): number { return query.execute("test", 1); }
export interface Unused { first: number; }
export interface Unused { second: number; }
interface Private { first: number; }
interface Private { second: number; }

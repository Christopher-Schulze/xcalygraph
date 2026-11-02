export class Service { run(): void {} }
export interface Bad { [key: string]: string; }
export interface Bad { service: Service; }
export function useBad(value: Bad): void { value.service.run(); }
export interface Duplicate { [key: string]: unknown; }
export interface Duplicate { [key: string]: unknown; }
export interface Duplicate { service: Service; }
export function useDuplicate(value: Duplicate): void { value.service.run(); }
export interface Mismatch { [key: string]: string; }
export interface Mismatch { [key: number]: number; }
export interface Mismatch { service: Service; }
export function useMismatch(value: Mismatch): void { value.service.run(); }

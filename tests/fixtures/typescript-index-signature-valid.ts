export interface Strings { [key: string]: string; }
export interface Strings { label: string; }
export class Service { run(): void {} }
export interface Bag { service: Service; }
export interface Bag { [key: string]: unknown; }
export interface Bag { [key: number]: unknown; }
export function useLabel(value: Strings): string { return value.label; }
export function useBag(value: Bag): void { value.service.run(); }

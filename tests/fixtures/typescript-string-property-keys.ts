export interface SpecialKeys {
  "a,b": string;
  "": boolean;
  "escaped\u0061": number;
  "line\nfeed": string;
  "\u{1F600}": boolean;
  "\uD800": string;
  "\\uD800": number;
}

export interface SeparateKeys {
  a: string;
  b: string;
}

declare const unique: unique symbol;
export interface ComputedKey {
  [unique]: string;
  regular: number;
}

export class Service { run(): void {} }
export class OtherService { run(): void {} }
export interface TypedKeys { "\u0063lient": Service; }
export function useTyped(s: TypedKeys): void { s.client.run(); }

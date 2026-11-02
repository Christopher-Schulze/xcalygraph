export class Service {
  run(): number {
    return 1;
  }
}

export interface EscapedField {
  \u0061: Service;
}

export interface EscapedField {
  "a": Service;
}

export function useEscapedField(input: EscapedField): number {
  return input.\u0061.run();
}

export interface UnicodeField {
  \u03B1: Service;
}

export function useUnicodeField(input: UnicodeField): number {
  return input.\u03B1.run();
}

export class ClassField {
  \u0062eta: Service = new Service();
}

export function useClassField(input: ClassField): number {
  return input.beta.run();
}

export interface EscapedMethod {
  \u0064o(): number;
}

export interface EscapedMethod {
  do(): number;
}

export function useEscapedMethod(input: EscapedMethod): number {
  return input.\u0064o();
}

export interface DistinctKeys {
  \u0061: string;
  "\\u0061": number;
}

export const distinctKeys: DistinctKeys = { a: "ok", "\\u0061": 1 };

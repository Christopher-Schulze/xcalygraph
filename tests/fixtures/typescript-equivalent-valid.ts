export interface Strings { value: number; }
export interface Strings { "value": number; }
export interface Escaped { a?: string; }
export interface Escaped { "\u0061"?: string; }
export interface Numbers { 0x10: string; }
export interface Numbers { "16": string; }
export function readValue(input: Strings): number { return input.value; }
export interface EscapedIdentifier { \u0061: string; }
export interface EscapedIdentifier { "\\u0061": string; }

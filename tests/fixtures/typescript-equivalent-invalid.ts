export interface DifferentTypes { value: string; }
export interface DifferentTypes { "value": number; }
export interface DifferentOptional { value?: string; }
export interface DifferentOptional { "value": string; }
export interface DifferentReadonly { readonly value: string; }
export interface DifferentReadonly { "value": string; }
export interface DifferentLiteral { value: "value"; }
export interface DifferentLiteral { "value": "different"; }

interface AliasBase { id: string; }
export { AliasBase };
export interface AliasChild extends AliasBase { left: number; }
export interface AliasChild extends AliasBase { right: number; }

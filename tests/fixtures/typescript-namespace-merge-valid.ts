export {};
export namespace Api {
  export interface Config { left: string; service: Service; }
  export class Service { run(): number { return 1; } }
  export function useFirst(value: Config): number { return value.left.length + value.service.run(); }
}
export namespace Api {
  export interface Config { right: number; }
  export function useSecond(value: Config): number { return value.left.length + value.right + value.service.run(); }
}
export namespace Other {
  export interface Config { other: boolean; }
}
export namespace Root {
  export namespace Nested {
    export interface Data { left: string; }
  }
}
export namespace Root.Nested {
  export interface Data { right: number; }
}

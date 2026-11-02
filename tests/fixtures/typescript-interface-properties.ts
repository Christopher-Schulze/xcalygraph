export interface Config {
  readonly width: number;
  height?: number;
  nested: { inner: string };
  choice: Other | Unrelated;
  "display-name": string;
  run(arg: string): boolean;
}

export interface Other {
  enabled: boolean;
}

export class Unrelated {
  other: number = 1;
}

export interface Config {
  readonly width: number;
  height?: number;
  nested: { inner: string };
  choice: Other | Unrelated;
  "display-name": string;
  'quoted-key': string;
  "ümlaut": string;
  123: boolean;
  9007199254740991: number;
  9007199254740992: number;
  "a,b": string;
  "": string;
  "escaped\u0061": string;
  "\uD800": string;
  "\\uD800": string;
  0x10: number;
  1e3: number;
  run(arg: string): boolean;
}

export interface Other {
  enabled: boolean;
}

export class Unrelated {
  other: number = 1;
}

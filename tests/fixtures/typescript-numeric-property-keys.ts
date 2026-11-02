export interface NumericKeys {
  0x10: string;
  1e3: number;
  0b1010: boolean;
  0o77: string;
  1_234: number;
  0x149ea49c2894c29: string;
  679496604817411.25: string;
  1e-7: number;
  1e-6: number;
  1e21: number;
  1e20: number;
  1e400: string;
  9007199254740993: boolean;
}

export interface AmbiguousDecimalKey {
  1.00000000000000011102230246251565404236316680908203125: number;
  safe: boolean;
}

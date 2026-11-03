export interface Left {
  common(): string;
  leftOnly(): void;
}

export interface Right {
  common(): string;
  rightOnly(): void;
}

export interface Empty {}

export interface CompositeConfig {
  choice: Left | Right;
  optional?: Left | Right;
  incomplete: Left | Empty;
  unavailable: Left | unknown;
  intersection: Empty & Left;
}

export function union_common(value: CompositeConfig): string {
  return value.choice.common();
}

export function optional_union(value: CompositeConfig): void {
  // @ts-expect-error the optional property may be absent.
  value.optional.common();
}

export function union_missing(value: CompositeConfig): void {
  // @ts-expect-error leftOnly is absent from one union arm.
  value.incomplete.leftOnly();
}

export function union_unavailable(value: CompositeConfig): void {
  // @ts-expect-error unknown makes the receiver unavailable.
  value.unavailable.leftOnly();
}

export function intersection_member(value: CompositeConfig): void {
  value.intersection.leftOnly();
}

export class CompositeHolder {
  value: Left[] = [];

  array_member(): void {
    // @ts-expect-error an array does not expose methods from its element type.
    this.value.leftOnly();
  }
}

class CompositeClass {
  value: Left | Right;
  optional?: Left | Right;

  constructor(value: Left | Right) {
    this.value = value;
  }
}

export function class_union_common(value: CompositeClass): string {
  return value.value.common();
}

export function class_optional_union(value: CompositeClass): void {
  // @ts-expect-error the optional property may be absent.
  value.optional.common();
}

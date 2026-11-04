import { Target } from 'missing';
export class Other {}
export function caller(value: Other): Target { return value as Target; }
export function wrong(value: Other): Other { return value; }

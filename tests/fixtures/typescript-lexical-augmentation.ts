import "./provider";
declare module "./provider" { interface Imported { extra(): number; } }
export function outsideAugmentation(value: Imported): number { return value.extra(); }

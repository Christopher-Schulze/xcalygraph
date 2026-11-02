import "./invalid";
declare module "./invalid" { interface AugmentedBase { extra: number; } }
declare module "./invalid" { interface Exposed { id: number; } }

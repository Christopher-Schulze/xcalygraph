interface Base { id: string; }
interface Broken extends Base { id: number; }
interface Broken extends Base { right: number; }
export interface Unknown extends MissingBase { left: number; }
export interface Unknown extends MissingBase { right: number; }
interface MethodBase { run(): number; }
interface BrokenMethod extends MethodBase { run(): string; }
interface BrokenMethod extends MethodBase { right: number; }
export interface AugmentedBase { id: string; }
export interface Augmented extends AugmentedBase { extra: string; }
export interface Augmented extends AugmentedBase { right: number; }
interface HiddenBase { id: string; }
export interface Exposed extends HiddenBase { left: number; }
export interface Exposed extends HiddenBase { right: number; }

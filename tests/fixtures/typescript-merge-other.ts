export interface Config<T> { other: T; }
import { Services } from "./model";
export function useImported(services: Services): number { return services.extra.run(); }

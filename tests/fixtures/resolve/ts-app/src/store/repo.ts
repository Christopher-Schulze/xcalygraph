/** Row storage contracts. */
export interface Repo {
  get(id: string): Row | undefined;
}

export type Row = { id: string };

/** Keeps rows in memory. */
export class MemoryRepo implements Repo {
  private rows = new Map<string, Row>();

  get(id: string): Row | undefined {
    return this.rows.get(id);
  }
}

export default function openRepo(): Repo {
  return new MemoryRepo();
}

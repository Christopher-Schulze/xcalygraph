export interface Item {
  title: string;
  date: string;
}

const items: Item[] = [];

export async function listItems(): Promise<Item[]> {
  return items;
}

export async function addItem(title: string): Promise<void> {
  items.push({ title, date: new Date().toISOString() });
}

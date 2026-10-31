import { addItem, listItems } from '$lib/items';

export async function load() {
  return { items: await listItems() };
}

export const actions = {
  default: async ({ request }: { request: Request }) => {
    await addItem(String((await request.formData()).get('title')));
  }
};

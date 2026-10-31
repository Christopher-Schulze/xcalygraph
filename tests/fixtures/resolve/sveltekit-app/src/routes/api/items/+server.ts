import { listItems } from '$lib/items';

export async function GET() {
  return Response.json(await listItems());
}

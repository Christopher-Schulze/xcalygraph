import { listProducts } from "@/lib/products"

export default async function handler(request: unknown, response: { json(body: unknown): void }) {
  response.json(await listProducts())
}

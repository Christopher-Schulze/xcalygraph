import { createProduct, listProducts } from "@/lib/products"

export async function GET() {
  return Response.json(await listProducts())
}

export async function POST(request: Request) {
  return Response.json(await createProduct(await request.json()))
}

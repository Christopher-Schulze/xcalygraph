"use server"

import { createProduct } from "@/lib/products"

export async function addToCart(id: string) {
  await createProduct({ id, name: "cart item" })
}

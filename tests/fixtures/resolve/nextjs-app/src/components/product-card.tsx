"use client"

import { addToCart } from "@/app/actions"

export function ProductCard({ product }: { product: { id: string; name: string } }) {
  return <button onClick={() => addToCart(product.id)}>{product.name}</button>
}

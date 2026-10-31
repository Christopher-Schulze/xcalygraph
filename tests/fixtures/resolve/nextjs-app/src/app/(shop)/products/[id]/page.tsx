import { ProductCard } from "@/components/product-card"
import { getProduct } from "@/lib/products"

export async function generateMetadata({ params }: { params: { id: string } }) {
  const product = await getProduct(params.id)
  return { title: product.name }
}

export default async function ProductPage({ params }: { params: { id: string } }) {
  const product = await getProduct(params.id)
  return <ProductCard product={product} />
}

export interface Product {
  id: string
  name: string
}

export async function getProduct(id: string): Promise<Product> {
  return { id, name: "product" }
}

export async function listProducts(): Promise<Product[]> {
  return [await getProduct("1")]
}

export async function createProduct(product: Product): Promise<Product> {
  return product
}

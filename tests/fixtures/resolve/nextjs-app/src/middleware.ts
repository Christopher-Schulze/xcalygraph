export function middleware(request: Request) {
  return request
}

export const config = { matcher: ["/products/:path*"] }

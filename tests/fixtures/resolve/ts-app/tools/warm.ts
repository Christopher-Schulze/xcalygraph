// A package importing its own subpath (`ts-app/cache/lru`): package exports point at build
// output, the source lives under `src/`.
import { Lru } from 'ts-app/cache/lru'

export function warm(): Lru {
  const cache = new Lru()
  cache.set('a')
  return cache
}

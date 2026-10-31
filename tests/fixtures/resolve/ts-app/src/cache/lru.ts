// ECMAScript private methods are definitions; `this.#evict()` calls them.
export class Lru {
  #limit = 8

  set(key: string): void {
    this.#evict(key)
  }

  #evict(key: string): void {
    void key
  }
}

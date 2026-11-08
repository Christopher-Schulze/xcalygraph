interface Handler {
  close(code: number): void;
}
export const handler: Handler = {
  close(code) {},
};
export const plain = {
  close(code) {},
};
export function run(h: Handler) {
  h.close(1);
}

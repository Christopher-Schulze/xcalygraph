export async function handle({ event, resolve }: { event: unknown; resolve: (event: unknown) => Response }) {
  return resolve(event);
}

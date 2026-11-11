/** One pending object click for the whole tree, cancelled by a second click or another interaction. */
export function createTreeClickNavigation(showObjects: () => void) {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let generation = 0;
  const cancel = () => {
    generation += 1;
    clearTimeout(timer);
    timer = undefined;
  };
  return {
    cancel,
    async singleClick(interval: Promise<number>) {
      cancel();
      const request = generation;
      const startedAt = Date.now();
      const milliseconds = await interval;
      if (request !== generation) return;
      timer = setTimeout(() => {
        timer = undefined;
        showObjects();
      }, Math.max(0, milliseconds - (Date.now() - startedAt)));
    },
  };
}

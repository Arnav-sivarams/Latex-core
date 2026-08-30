export const AUTO_BUILD_IDLE_MS = 2000;

export function createIdleBuildScheduler({
  requestBuild,
  setTimer = globalThis.setTimeout.bind(globalThis),
  clearTimer = globalThis.clearTimeout.bind(globalThis),
  delay = AUTO_BUILD_IDLE_MS,
}) {
  let timer = null;
  return {
    durableUpdate() {
      if (timer !== null) clearTimer(timer);
      timer = setTimer(async () => {
        timer = null;
        await requestBuild('auto');
      }, delay);
    },
    cancel() {
      if (timer !== null) clearTimer(timer);
      timer = null;
    },
    pending() { return timer !== null; },
  };
}

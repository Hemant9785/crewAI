import { runContract } from './testing/contract.js';
import { MemoryStore } from './memoryBackend.js';

runContract('memory', () => {
  const clock = { t: 1_000_000 };
  const store = new MemoryStore(() => clock.t);
  let n = 0;
  return {
    user: async () => store.backendFor(`u${++n}`),
    advance: (ms: number) => {
      clock.t += ms;
    },
  };
});

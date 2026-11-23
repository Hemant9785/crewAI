import { defineConfig } from 'vitest/config';
export default defineConfig({
  test: {
    include: ['src/**/*.test.{ts,tsx}'],
    exclude: ['src/**/*.rules.test.ts', 'src/**/*.live.test.ts'],
    testTimeout: 15000,
    // Ink draws only the last frame when it detects a CI environment. The screen tests need every frame, as in a
    // person's terminal, so switch that detection off (it reads CI at import time, so it must be set here).
    env: { CI: '0' },
  },
});

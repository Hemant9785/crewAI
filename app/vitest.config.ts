import { defineConfig } from 'vitest/config';
export default defineConfig({
  test: { include: ['src/**/*.test.{ts,tsx}'], exclude: ['src/**/*.rules.test.ts', 'src/**/*.live.test.ts'], testTimeout: 15000 },
});

import { defineConfig } from 'vitest/config';
export default defineConfig({
  test: { include: ['src/**/*.test.{ts,tsx}'], exclude: ['src/**/*.rules.test.ts'], testTimeout: 15000 },
});

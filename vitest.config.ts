import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    include: ['packages/**/tests/**/*.spec.ts', 'tests/**/*.spec.ts'],
    environment: 'node',
    coverage: {
      provider: 'v8',
      include: ['packages/**/src/**'],
      exclude: ['packages/**/src/data/**', '**/*.json'],
      reporter: ['text', 'text-summary'],
      thresholds: {
        // Ratchet: current floor of the deterministic pipeline packages.
        // Raise, never lower. Entity-validate/state-machine/contract code is
        // pure logic and must stay covered.
        statements: 80,
        branches: 70,
        functions: 80,
        lines: 80,
      },
    },
  },
})

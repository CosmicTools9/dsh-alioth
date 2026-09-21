import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    include: ['packages/**/tests/**/*.spec.ts', 'tests/**/*.spec.ts'],
    environment: 'node',
    coverage: {
      provider: 'v8',
      include: ['packages/**/src/**'],
      // The vendored framework tree nests its own `src/` dirs (Rust crates, .mjs
      // gate scripts) which the `packages/**/src/**` glob would sweep in; V8
      // coverage then fails to parse them and prints one RollupError per file.
      // Vendor provenance is gated by `check:vendor`, not by coverage.
      exclude: ['packages/**/src/data/**', '**/vendor/**', '**/*.json'],
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

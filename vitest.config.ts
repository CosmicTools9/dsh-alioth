import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    include: ['packages/**/tests/**/*.spec.ts', 'tests/**/*.spec.ts'],
    environment: 'node',
    // Upper bounds, not budgets: the DB-backed suites create a throwaway database per suite and
    // bootstrap the registry (the vendored baseline seeds ~27k rows) inside their hooks, so the
    // vitest defaults (5s test / 10s hook) turn a busy dev machine into red gates — observed at
    // load ~70 from unrelated builds, where CREATE DATABASE plus the baseline insert alone exceeds
    // them. CI (an idle runner) is unaffected: a passing test never waits this long.
    testTimeout: 60_000,
    hookTimeout: 60_000,
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

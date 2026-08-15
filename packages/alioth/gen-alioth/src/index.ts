/**
 * Alioth artifact contracts and generators. Pure, no I/O, no harness deps:
 * consumed by tools (`tool-alioth`, future write tools) and tests as the
 * validation ground for generated artifacts.
 * @module @dsh-alioth/gen-alioth
 */

export { validateArtifact, type ArtifactKind, type ValidationResult } from './validate.ts'
export { generateApp, generateExtensions, generateExtension, sourceModuleDirs, EXTENSION_FILES, type AppSpec, type GeneratedApp, type ModuleSpec, type NavigationGroup } from './generate.ts'

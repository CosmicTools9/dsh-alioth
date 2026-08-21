/**
 * Alioth artifact contracts and generators. Pure, no I/O, no harness deps:
 * consumed by tools (`tool-alioth`, future write tools) and tests as the
 * validation ground for generated artifacts.
 * @module @dsh-alioth/gen-alioth
 */

export { validateArtifact, validateArtifactWith, type ArtifactKind, type ArtifactSchemas, type ValidationResult } from './validate.ts'
export { generateApp, generateModule, generateExtensions, generateExtension, generateService, sourceModuleDirs, sourceServiceDirs, EXTENSION_FILES, type AppSpec, type GeneratedApp, type ModuleSpec, type NavigationGroup, type ServiceSpec, type ServiceEntitySpec } from './generate.ts'

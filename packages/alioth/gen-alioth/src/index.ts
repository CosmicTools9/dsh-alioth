export { generateExtensions, type ExtensionPlanInput } from './extension-plan.ts'
/**
 * Alioth artifact contracts and generators. Pure, no I/O, no harness deps:
 * consumed by tools (`tool-alioth`, future write tools) and tests as the
 * validation ground for generated artifacts.
 * @module @dsh-alioth/gen-alioth
 */

export { validateArtifact, validateArtifactWith, type ArtifactKind, type ArtifactSchemas, type ValidationResult } from './validate.ts'
export { generateApp, generateModule, generateBlock, generateService, generateNamespaceWorkspace, generateServiceCrate, sourceModuleDirs, sourceServiceDirs, EXTENSION_FILES, type AppSpec, type GeneratedApp, type ModuleSpec, type BlockSpec, type NavigationGroup, type ServiceSpec, type ServiceEntitySpec } from './generate.ts'
export { compareModelVersions, DEFAULT_MIN_ALIOTH_VERSION, displayModelVersion, modelVersionAnchor, parseModelVersion, satisfiesModelVersion } from './version.ts'

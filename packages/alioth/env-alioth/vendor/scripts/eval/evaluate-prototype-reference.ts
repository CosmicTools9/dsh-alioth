#!/usr/bin/env bun
/**
 * evaluate-prototype-reference.ts
 *
 * Static evaluator: score a generated Alioth prototype (`m-v{N}.html`, `b-v{N}.html`,
 * `a-v{N}.html`) against the Gateway Shell reference contract.
 *
 * Checks dimensions that can be evaluated without a browser:
 *   - Structural Shell (boot skeleton, #root, vendor scripts, layout variables)
 *   - Resource References (local fonts, tailwind-utilities.css, no CDN)
 *   - CSS Tokens (:root variables, no stray hardcoded colors)
 *   - Prohibited Patterns (no gl-gateway-* classes, no duplicated shell CSS)
 *   - Build Metadata (scene list, version comment)
 *   - Existing Gate Compliance (check-prototype-standalone, audit-css-framework)
 *
 * Visual and functional parity MUST be verified separately with:
 *   bash scripts/check/check-visual-verify.sh <prototype.html>
 *
 * Usage:
 *   bun scripts/eval/evaluate-prototype-reference.ts <prototype.html>
 *   bun scripts/eval/evaluate-prototype-reference.ts <prototype.html> --human
 */

import { readFileSync, existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { html } from '../lib/parsers.ts';
import type { CheerioAPI } from 'cheerio';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const ROOT = resolve(__dirname, '../..');
const REFERENCE_PATH = resolve(ROOT, '.agents/skills/alioth-design/references/gateway-shell.tsx');
const RUST_STANDALONE = resolve(ROOT, 'target/debug/ontology-mapping');
const CHECK_SCRIPTS = {
  cssFramework: resolve(ROOT, 'scripts/check/audit-css-framework.mjs'),
  classNames: resolve(ROOT, 'scripts/check/check-class-names.js'),
};
type Severity = 'error' | 'warning';
type Issue = { message: string; severity: Severity; dimension: string };
type DimensionResult = { dimension: string; score: number; max: number; issues: Issue[] };
type Report = {
  prototype: string;
  reference: string;
  weightedScore: number;
  pass: boolean;
  dimensions: DimensionResult[];
  recommendations: string[];
};

const WEIGHTS: Record<string, number> = {
  'Structural Shell': 0.25,
  'Resource References': 0.15,
  'CSS Tokens': 0.2,
  'Prohibited Patterns': 0.15,
  'Build Metadata': 0.05,
  'Existing Gate Compliance': 0.2,
};

function main() {
  const filePath = process.argv[2];
  const human = process.argv.includes('--human');
  if (!filePath) {
    console.error('Usage: bun scripts/eval/evaluate-prototype-reference.ts <prototype.html>');
    process.exit(1);
  }

  const htmlPath = resolve(filePath);
  if (!existsSync(htmlPath)) {
    console.error('File not found: ' + htmlPath);
    process.exit(1);
  }

  const htmlText = readFileSync(htmlPath, 'utf-8');
  const $ = html.load(htmlText);

  const dimensions: DimensionResult[] = [
    evaluateStructuralShell($, htmlText),
    evaluateResourceReferences($, htmlText),
    evaluateCssTokens($, htmlText),
    evaluateProhibitedPatterns($, htmlText),
    evaluateBuildMetadata(htmlText),
    evaluateExistingGates(htmlPath),
  ];

  let totalWeight = 0;
  let weightedSum = 0;
  for (const d of dimensions) {
    const w = WEIGHTS[d.dimension] || 0;
    totalWeight += w;
    weightedSum += (d.score / d.max) * 100 * w;
  }
  const weightedScore = totalWeight > 0 ? weightedSum / totalWeight : 0;
  const pass = weightedScore >= 90 && dimensions.every((d) => d.score >= d.max * 0.6);

  const recommendations: string[] = [];
  for (const d of dimensions) {
    for (const i of d.issues) {
      if (i.severity === 'error') {
        recommendations.push(`[${d.dimension}] ${i.message}`);
      }
    }
  }
  if (recommendations.length === 0) {
    recommendations.push(
      'Static gate passed. Run visual verification next: bash scripts/check/check-visual-verify.sh ' +
        htmlPath,
    );
  }

  const report: Report = {
    prototype: htmlPath,
    reference: REFERENCE_PATH,
    weightedScore: Math.round(weightedScore * 10) / 10,
    pass,
    dimensions,
    recommendations,
  };

  if (human) {
    printHuman(report);
  } else {
    console.log(JSON.stringify(report, null, 2));
  }
  process.exit(pass ? 0 : 1);
}

function evaluateStructuralShell($: CheerioAPI, htmlText: string): DimensionResult {
  const issues: Issue[] = [];

  if ($('#boot-skeleton').length === 0) {
    issues.push({
      message: 'Missing #boot-skeleton loading placeholder',
      severity: 'error',
      dimension: 'Structural Shell',
    });
  } else {
    const boot = $('#boot-skeleton');
    if (boot.find('.boot-skeleton-sidebar').length === 0) {
      issues.push({
        message: 'Boot skeleton missing sidebar placeholder',
        severity: 'warning',
        dimension: 'Structural Shell',
      });
    }
    if (boot.find('.boot-skeleton-main').length === 0) {
      issues.push({
        message: 'Boot skeleton missing main placeholder',
        severity: 'warning',
        dimension: 'Structural Shell',
      });
    }
    if (boot.find('.boot-skeleton-loader').length === 0) {
      issues.push({
        message: 'Boot skeleton missing loader indicator',
        severity: 'warning',
        dimension: 'Structural Shell',
      });
    }
  }

  if ($('#root').length === 0) {
    issues.push({
      message: 'Missing #root mount target',
      severity: 'error',
      dimension: 'Structural Shell',
    });
  }

  const reactScripts = $('script[src*="vendor/react"]').length;
  if (reactScripts < 3) {
    issues.push({
      message: 'Expected at least 3 vendor React scripts, found ' + reactScripts,
      severity: 'warning',
      dimension: 'Structural Shell',
    });
  }

  const requiredVars = ['--topbar-height', '--sidebar-width', '--sidebar-collapsed-width'];
  const hasExternalBase = htmlText.includes('prototype-base.css');
  for (const v of requiredVars) {
    if (
      !hasExternalBase &&
      !htmlText.includes(' ' + v + ':') &&
      !htmlText.includes('\n' + v + ':')
    ) {
      issues.push({
        message: 'Missing CSS variable ' + v,
        severity: 'warning',
        dimension: 'Structural Shell',
      });
    }
  }

  const score = Math.max(
    0,
    5 -
      issues.filter((i) => i.severity === 'error').length * 1.5 -
      issues.filter((i) => i.severity === 'warning').length * 0.5,
  );
  return { dimension: 'Structural Shell', score, max: 5, issues };
}

function evaluateResourceReferences($: CheerioAPI, htmlText: string): DimensionResult {
  const issues: Issue[] = [];

  const linkTags = $('link[rel="stylesheet"]');
  const hasInterFont = linkTags
    .toArray()
    .some((el) => $(el).attr('href')?.includes('inter.css') ?? false);
  const hasJetbrainsFont = linkTags
    .toArray()
    .some((el) => $(el).attr('href')?.includes('jetbrains-mono.css') ?? false);
  if (!hasInterFont) {
    issues.push({
      message: 'Missing Inter font reference',
      severity: 'warning',
      dimension: 'Resource References',
    });
  }
  if (!hasJetbrainsFont) {
    issues.push({
      message: 'Missing JetBrains Mono font reference',
      severity: 'warning',
      dimension: 'Resource References',
    });
  }

  const hasBaseCss =
    htmlText.includes('prototype-base.css') ||
    htmlText.includes('tailwind-utilities.css') ||
    htmlText.includes('--background:');
  if (!hasBaseCss) {
    issues.push({
      message:
        'Missing prototype-base.css (or legacy tailwind-utilities.css) reference or design tokens',
      severity: 'error',
      dimension: 'Resource References',
    });
  }

  const forbiddenCdn = [
    'fonts.googleapis.com',
    'fonts.gstatic.com',
    'cdn.jsdelivr.net',
    'unpkg.com',
    'cdnjs.cloudflare.com',
  ];
  const lower = htmlText.toLowerCase();
  for (const cdn of forbiddenCdn) {
    if (lower.includes(cdn)) {
      issues.push({
        message: 'Forbidden external CDN reference: ' + cdn,
        severity: 'error',
        dimension: 'Resource References',
      });
    }
  }

  const score = Math.max(
    0,
    5 -
      issues.filter((i) => i.severity === 'error').length * 1.5 -
      issues.filter((i) => i.severity === 'warning').length * 0.5,
  );
  return { dimension: 'Resource References', score, max: 5, issues };
}

function evaluateCssTokens($: CheerioAPI, htmlText: string): DimensionResult {
  const issues: Issue[] = [];
  const styleBlocks = $('style')
    .toArray()
    .map((el) => $(el).html() || '');
  const allCss = styleBlocks.join('\n');
  const hasExternalBase = htmlText.includes('prototype-base.css');

  if (!hasExternalBase && !allCss.includes(':root')) {
    issues.push({
      message: 'No :root CSS variables block found',
      severity: 'error',
      dimension: 'CSS Tokens',
    });
  }

  if (!hasExternalBase) {
    const requiredTokens = [
      '--background',
      '--foreground',
      '--primary',
      '--secondary',
      '--muted',
      '--border',
      '--card',
    ];
    for (const t of requiredTokens) {
      if (!allCss.includes(t)) {
        issues.push({
          message: 'Missing token ' + t,
          severity: 'warning',
          dimension: 'CSS Tokens',
        });
      }
    }
  }

  // Hardcoded hex/rgba colors outside of data URLs and SVG paths
  const suspiciousColor = /#[0-9a-fA-F]{3,8}\b|rgba?\s*\(/g;
  let match: RegExpExecArray | null;
  const allowedHardcodes = [
    '#ff3b30',
    '#ffffff',
    '#000000',
    '#0f172a',
    '#f8fafc',
    '#18181b',
    '#fafafa',
    '#7c3aed',
    '#09090b',
  ];
  while ((match = suspiciousColor.exec(allCss)) !== null) {
    const value = match[0].toLowerCase();
    if (match[0].startsWith('rgb')) continue;
    if (!allowedHardcodes.includes(value)) {
      issues.push({
        message: 'Suspicious hardcoded color in CSS: ' + match[0],
        severity: 'warning',
        dimension: 'CSS Tokens',
      });
      // cap warnings to avoid flooding
      if (issues.filter((i) => i.message.startsWith('Suspicious hardcoded color')).length >= 5)
        break;
    }
  }

  const score = Math.max(
    0,
    5 -
      issues.filter((i) => i.severity === 'error').length * 1.5 -
      issues.filter((i) => i.severity === 'warning').length * 0.3,
  );
  return { dimension: 'CSS Tokens', score, max: 5, issues };
}

function evaluateProhibitedPatterns($: CheerioAPI, htmlText: string): DimensionResult {
  const issues: Issue[] = [];

  const classNames = new Set<string>();
  $('[class]').each((_, el) => {
    const cls = $(el).attr('class');
    if (cls) {
      for (const c of cls.split(/\s+/)) classNames.add(c);
    }
  });
  for (const c of classNames) {
    if (c.startsWith('gl-gateway-')) {
      issues.push({
        message: 'Prohibited gl-gateway-* class name in HTML: ' + c,
        severity: 'error',
        dimension: 'Prohibited Patterns',
      });
    }
  }

  const styleBlocks = $('style')
    .toArray()
    .map((el) => $(el).html() || '');
  for (const block of styleBlocks) {
    if (block.includes('.gl-gateway-')) {
      issues.push({
        message: 'Prohibited gl-gateway-* CSS selector in <style>',
        severity: 'error',
        dimension: 'Prohibited Patterns',
      });
    }
  }

  if (htmlText.includes('react-router')) {
    issues.push({
      message: 'Prohibited dependency: react-router',
      severity: 'error',
      dimension: 'Prohibited Patterns',
    });
  }
  if (htmlText.includes('scrollIntoView')) {
    issues.push({
      message: 'Prohibited API: scrollIntoView',
      severity: 'warning',
      dimension: 'Prohibited Patterns',
    });
  }

  const score = Math.max(0, 5 - issues.filter((i) => i.severity === 'error').length * 2.5);
  return { dimension: 'Prohibited Patterns', score, max: 5, issues };
}

function evaluateBuildMetadata(htmlText: string): DimensionResult {
  const issues: Issue[] = [];

  const firstLine = htmlText.split('\n')[0] || '';
  const moduleMatch = firstLine.match(/Module\s+m-v(\d+)/);
  const blockMatch = firstLine.match(/Block\s+b-v(\d+)/);
  const appMatch = firstLine.match(/App\s+a-v(\d+)/);
  if (!moduleMatch && !blockMatch && !appMatch) {
    issues.push({
      message: 'First-line version metadata missing (expected <!-- Module m-vN ... --> or similar)',
      severity: 'warning',
      dimension: 'Build Metadata',
    });
  }

  if (firstLine.includes('scenes: ?') || firstLine.includes('scenes: ? |')) {
    issues.push({
      message: 'Build metadata contains unresolved scenes: "?"',
      severity: 'error',
      dimension: 'Build Metadata',
    });
  } else if (firstLine.includes('scenes:')) {
    const sceneList = firstLine.match(/scenes:\s*([^|]+)/)?.[1]?.trim() ?? '';
    if (sceneList.length === 0) {
      issues.push({
        message: 'Scene list empty in metadata',
        severity: 'warning',
        dimension: 'Build Metadata',
      });
    }
  }

  const score = Math.max(
    0,
    5 -
      issues.filter((i) => i.severity === 'error').length * 2 -
      issues.filter((i) => i.severity === 'warning').length * 1,
  );
  return { dimension: 'Build Metadata', score, max: 5, issues };
}

function evaluateExistingGates(htmlPath: string): DimensionResult {
  const issues: Issue[] = [];

  const run = (cmd: string, args: string[], label: string, opts?: { warningExitCodes?: number[] }) => {
    const r = spawnSync(cmd, args, { encoding: 'utf-8', timeout: 60000 });
    if (r.status !== 0) {
      const isWarning = opts?.warningExitCodes?.includes(r.status ?? 0) ?? false;
      const severity = isWarning ? 'warning' : 'error';
      issues.push({
        message: label + ' ' + (isWarning ? 'warnings' : 'failed') + ' (exit ' + r.status + ')',
        severity,
        dimension: 'Existing Gate Compliance',
      });
      if (r.stderr) {
        for (const line of r.stderr.split('\n').slice(0, 3)) {
          if (line.trim())
            issues.push({
              message: '  ' + line.trim(),
              severity: 'warning',
              dimension: 'Existing Gate Compliance',
            });
        }
      }
    }
  };

  // Rust CLI: target/debug/ontology-mapping prototype-check <html>
  if (existsSync(RUST_STANDALONE)) {
    run(RUST_STANDALONE, ['prototype-check', htmlPath], 'ontology-mapping prototype-check', { warningExitCodes: [2] });
  } else {
    issues.push({
      message:
        'ontology-mapping binary not found at ' + RUST_STANDALONE + ' (skip standalone check)',
      severity: 'warning',
      dimension: 'Existing Gate Compliance',
    });
  }

  run('bun', [CHECK_SCRIPTS.cssFramework, htmlPath], 'audit-css-framework.mjs');
  // check-class-names.js is noisy for generated ESM prototypes; report only as warning
  const classCheck = spawnSync('bun', [CHECK_SCRIPTS.classNames, htmlPath], {
    encoding: 'utf-8',
    timeout: 60000,
  });
  if (classCheck.status !== 0) {
    issues.push({
      message:
        'check-class-names.js reported missing classes (often false positives for Tailwind utilities)',
      severity: 'warning',
      dimension: 'Existing Gate Compliance',
    });
  }

  const score = Math.max(
    0,
    5 -
      issues.filter((i) => i.severity === 'error').length * 1.5 -
      issues.filter((i) => i.severity === 'warning').length * 0.5,
  );
  return { dimension: 'Existing Gate Compliance', score, max: 5, issues };
}

function printHuman(report: Report) {
  const status = report.pass ? '✅ PASS' : '❌ FAIL';
  console.log('Prototype Reference Evaluation');
  console.log('  Prototype: ' + report.prototype);
  console.log('  Reference: ' + report.reference);
  console.log('  Score:     ' + report.weightedScore + ' / 100 ' + status);
  console.log('');
  console.log('Dimensions:');
  for (const d of report.dimensions) {
    const icon = d.score >= d.max * 0.8 ? '✅' : d.score >= d.max * 0.6 ? '⚠️' : '❌';
    console.log('  ' + icon + ' ' + d.dimension + ': ' + d.score + '/' + d.max);
    for (const i of d.issues) {
      const prefix = i.severity === 'error' ? '  ❌' : '  ⚠️';
      console.log('    ' + prefix + ' ' + i.message);
    }
  }
  console.log('');
  console.log('Recommendations:');
  for (const r of report.recommendations) {
    console.log('  - ' + r);
  }
}

main();

## Summary

<!-- What changed and why. One paragraph. -->

## Verification

<!-- Evidence that this change works. List the exact commands you ran:
     pnpm run test, mise run gates, docker run --check, manual dialogue, ... -->

- [ ] `pnpm run test` green
- [ ] `mise run gates` (or CI) green
- [ ] Model-visible surface changed? → refreshed `tests/__snapshots__/model-surface.json` via `UPDATE_SNAPSHOTS=1` and reviewed the diff
- [ ] Vendor tree changed? → regenerated `vendor/PROVENANCE.json` via `check:vendor --update`
- [ ] Semantic dicts changed? → ran `generate-semantic-dicts.ts` (anchor refreshed)

## Scope check

<!-- Anything out of scope deliberately NOT done? Anything risky? -->

# Plan: Upgrade GitHub Actions to Latest Versions

## Summary

Upgrade all GitHub Actions in `.github/workflows/ci.yml` and `.github/workflows/release.yml` to their latest major versions to resolve the Node.js 20 deprecation warning (`actions/upload-artifact@v4` is forced to run on Node.js 24) and keep all actions current.

## Current State Analysis

### Actions used and their versions

| Action | Current | Latest | Runtime | Needs Upgrade? |
|--------|---------|--------|---------|----------------|
| `actions/checkout` | v5 | **v7.0.0** (Jun 18, 2026) | node24 (v5+) | Yes → v7 |
| `pnpm/action-setup` | v5 | **v6** (v6.0.5+) | node24 | Yes → v6 |
| `actions/setup-node` | v5 | **v6.4.0** (Apr 20, 2026) | node24 | Yes → v6 |
| `actions/upload-artifact` | v4 | **v7.0.1** | node24 (v5+) | Yes → v7 |
| `actions/download-artifact` | v4 | **v5.0.0** | node24 | Yes → v5 |
| `dtolnay/rust-toolchain` | stable | stable | composite (N/A) | No |
| `Swatinem/rust-cache` | v2 | v2.9.1 | **node24 already** | No |
| `taiki-e/install-action` | v2 | v2.82.10 | composite (N/A) | No |
| `softprops/action-gh-release` | v2 | **v3.0.0** (Jun 10, 2026) | node24 | Yes → v3 |

### Key findings
- **Root cause**: `actions/upload-artifact@v4` uses `node20` runtime, triggering the deprecation warning.
- **Swatinem/rust-cache@v2** (v2.9.1) already uses `node24` — confirmed via `action.yml` (`runs: using: "node24"`).
- **dtolnay/rust-toolchain** and **taiki-e/install-action** are composite actions — no Node.js runtime, no upgrade needed.
- **No breaking changes** affect our usage: all `with:` parameters remain the same across the upgraded versions.
  - `upload-artifact` v5-v7: API identical to v4 for our `name`/`path` usage.
  - `download-artifact` v5: breaking change only affects single-artifact downloads by ID (we download by name).
  - `pnpm/action-setup` v6.0.3+ fixed a bug where `version: 10` installed pnpm v11 beta; `@v6` tag points to the fixed version.
  - `softprops/action-gh-release` v3: only change is Node 20→24 runtime; API unchanged.

## Proposed Changes

### File 1: `.github/workflows/ci.yml`

| Line context | Old | New |
|---|---|---|
| Checkout (frontend job) | `actions/checkout@v5` | `actions/checkout@v7` |
| Setup pnpm | `pnpm/action-setup@v5` | `pnpm/action-setup@v6` |
| Setup Node | `actions/setup-node@v5` | `actions/setup-node@v6` |
| Upload web dist | `actions/upload-artifact@v4` | `actions/upload-artifact@v7` |
| Checkout (rust job) | `actions/checkout@v5` | `actions/checkout@v7` |
| Download web dist | `actions/download-artifact@v4` | `actions/download-artifact@v5` |
| dtolnay/rust-toolchain | `@stable` | no change |
| Swatinem/rust-cache | `@v2` | no change |

### File 2: `.github/workflows/release.yml`

| Line context | Old | New |
|---|---|---|
| Checkout | `actions/checkout@v5` | `actions/checkout@v7` |
| Setup pnpm | `pnpm/action-setup@v5` | `pnpm/action-setup@v6` |
| Setup Node | `actions/setup-node@v5` | `actions/setup-node@v6` |
| dtolnay/rust-toolchain | `@stable` | no change |
| Swatinem/rust-cache | `@v2` | no change |
| taiki-e/install-action | `@v2` | no change |
| Upload to release | `softprops/action-gh-release@v2` | `softprops/action-gh-release@v3` |

## Assumptions & Decisions

1. **Node.js build version stays at 22**: The `node-version: 22` in `actions/setup-node` is for building the frontend, not the Actions runtime. Node.js 22 is still LTS. No change needed.
2. **pnpm version stays at 10**: The workflow explicitly sets `version: 10`. Upgrading pnpm itself to v11 is out of scope.
3. **No `with:` parameter changes**: All upgraded actions maintain backward-compatible APIs for our usage patterns.
4. **Use major version tags** (e.g., `@v7`, `@v6`): Consistent with the existing convention in the repo.

## Verification Steps

1. Run `git diff` to confirm only version strings changed, no accidental parameter modifications.
2. Push the branch and trigger CI — verify:
   - Frontend job: pnpm install, build, typecheck, artifact upload all succeed.
   - Rust job: artifact download, fmt check, clippy, tests all succeed.
   - No Node.js 20 deprecation warnings in the workflow run logs.
3. (Optional) Tag a test release to verify `release.yml` builds and uploads artifacts correctly.

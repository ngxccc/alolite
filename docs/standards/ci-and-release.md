# CI, Release & Automation Standards

## 1. Workflow Ownership

Each workflow under `.github/workflows/` owns one responsibility. A check runs in one workflow and nowhere else; other workflows reuse it via `workflow_call` or consume its artifact.

| File                | Owns                                                                                                                                            |
| :------------------ | :---------------------------------------------------------------------------------------------------------------------------------------------- |
| `ci.yml`            | Rust core/CLI format, clippy, and unit/integration tests; Desktop UI checks/build; Tauri release check (`cargo check --release --all-targets`). |
| `build.yml`         | Linux `.deb`, `.rpm`, `.AppImage`, and `.tar.gz` package builds (`packaging/`).                                                                 |
| `docker-ci.yml`     | GHCR CI container image build (`packaging/docker/**`) for `desktop-check` and `build`.                                                          |
| `docs.yml`          | `mdbook` build and GitHub Pages deployment for `docs/`.                                                                                         |
| `security.yml`      | Trivy filesystem scan and scheduled/PR security auditing.                                                                                       |
| `release.yml`       | Tag detection (`v*`), binary/package verification, and GitHub Release publishing.                                                               |
| `labeler.yml`       | `area:*` PR labels from `.github/labeler.yml`.                                                                                                  |
| `pr-type-label.yml` | `type:*` PR labels from Conventional Commit PR titles.                                                                                          |

- Do not duplicate quality commands across workflows; `release.yml` and `build.yml` reuse `ci.yml` or focus on packaging/publishing.
- Keep `name:` short and stable. Renaming a workflow or job renames its status check and breaks branch protection.

## 2. Area & Type Labels

- `area:*` describes which workspace or crate a pull request modifies and is applied automatically by `labeler.yml` from `.github/labeler.yml` (`area:protocol`, `area:store`, `area:mcp`, `area:core`, `area:cli`, `area:ui`).
- `type:*` (`type:feat`, `type:fix`, `type:perf`, `type:refactor`, `type:docs`, `type:test`, `type:chore`, `type:ci`, `type:style`) is synced from the PR title prefix by `pr-type-label.yml`.

## 3. Pipeline (`ci.yml`)

`ci.yml` runs on pull requests and pushes to `main` (ignoring `**/*.md`, `docs/**`, `.gitignore`, `LICENSE`, `.vscode/**`, `assets/**`, `.github/assets/**`):

| Job             | Commands                                                                                                                                                       |
| :-------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `core-cli`      | `cargo fmt --all -- --check`, `cargo clippy -p alolite-core -p alolite-cli --all-targets -- -D warnings`, `cargo test -p alolite-core -p alolite-cli --locked` |
| `frontend`      | `bun scripts/version.ts check`, `bun install --frozen-lockfile`, `bun run check`, `bun test`, `bun run build` (uploads `apps/desktop/dist`)                    |
| `desktop-check` | Downloads `desktop-dist` and runs `cargo check --release --all-targets` inside the GHCR CI container                                                           |

- `docs.yml` triggers separately on `docs/**` changes so documentation PRs build `mdbook` without waiting for full Rust/Tauri builds.
- `build.yml` triggers on packaging, Tauri config, and manifest changes (`packaging/**`, `Cargo.toml`, `Cargo.lock`, `apps/desktop/package.json`).

## 4. Required Checks & Branch Protection

Branch protection for `main` is repository configuration, not YAML. Require a pull request, require branches to be up to date, require conversation resolution, and leave force pushes and deletions disabled.

- `CI / Rust Core & CLI`, `CI / Frontend Desktop UI`, `CI / Desktop Tauri Check`, and `Security Scan / Trivy` are the core required checks.
- A workflow or job rename invalidates its required check; update branch protection in the same change window.

## 5. Release

1. Version consistency across `Cargo.toml`, `crates/*/Cargo.toml`, `apps/desktop/package.json`, and `crates/alolite-desktop/tauri.conf.json` is verified by `bun scripts/version.ts check`.
2. Pushing a `v*` tag (or dispatching `release.yml` with `tag_name`) runs `release.yml` to build and publish GitHub Release artifacts.
3. Re-running a release is idempotent; never delete or rewrite an existing published tag or release.

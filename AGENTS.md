# AGENTS.md

Operating guide for AI agents and humans working in this repository.

## Project

`alolite` is a lightweight native desktop messaging client organized as a clean Cargo workspace (`alolite-core` + `alolite-cli` + `apps/desktop` Tauri v2 + TypeScript), zero Node.js runtime, zero Electron/Chromium.

- Repository: <https://github.com/ngxccc/alolite>
- Rust 1.100+ / edition 2021
- All documentation (`docs/`, `README.md`, `docs/standards/`) is written completely in English.

## Commands

| Task                         | Command                                                                   |
| :--------------------------- | :------------------------------------------------------------------------ |
| Build CLI & core             | `cargo build`                                                             |
| Build Desktop app            | `(cd apps/desktop && bun run build) && cargo build --bin alolite-desktop` |
| Build Release binaries       | `cargo build --release --bin alolite --bin alolite-desktop`               |
| Test Rust core & CLI         | `cargo test --locked`                                                     |
| Test Desktop UI              | `(cd apps/desktop && bun run typecheck && bun test && bun run build)`     |
| Format                       | `cargo fmt --all`                                                         |
| Lint                         | `cargo clippy --workspace --all-targets -- -D warnings`                   |
| Run Desktop                  | `cargo run --bin alolite-desktop`                                         |
| Run MCP (stdio)              | `cargo run --bin alolite -- --mcp`                                        |
| Run headless REST + MCP HTTP | `cargo run --bin alolite -- --api-port 18128 --api-only`                  |

## Architecture Map

| Path                      | Responsibility                                                                                     |
| :------------------------ | :------------------------------------------------------------------------------------------------- |
| `crates/alolite-core/`    | UI-independent `CoreEngine`, `protocol/`, `storage/`, `mcp/`, `services/`, and `common/`           |
| `assets/`                 | Shared desktop icon (`app_icon.png`, `.ico`, `.svg`) and branding assets                           |
| `crates/alolite-cli/`     | `alolite` binary (`--mcp`, `--api-only`, `--qr`, `--phone`, desktop launcher)                      |
| `crates/alolite-desktop/` | Tauri v2 native desktop host (`alolite-desktop`) over `alolite-core` embedding `apps/desktop/dist` |
| `apps/desktop/`           | Pure feature-based TypeScript + React frontend (`src/{app,shared,features}/`)                      |
| `docs/`                   | Engineering standards, ADRs (`docs/adr/`), `book.toml`, `ROADMAP.md`, `LEGAL.md`                   |
| `packaging/`              | Linux `.deb`, `.rpm`, `.AppImage`, `.tar.gz` packaging scripts                                     |

### Protocol Submodule Map (`crates/alolite-core/src/protocol/`)

| Module                                                                                      | Single Responsibility                                                                                                                                                                                                        |
| :------------------------------------------------------------------------------------------ | :--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `endpoints.rs` & `registry/{mod,http_table,ws}.rs`                                          | SSOT for all host URLs (`SERVICE_MAP_V3`, fallbacks) + sorted 437 HTTP `commandId` catalog & WS `cmd` constants                                                                                                              |
| `wire.rs`, `crypto.rs`, `param_cipher.rs`, `socket.rs`                                      | Shared JSON wire helpers (`require_login`, `unwrap_data`, `pick_str`, `iter_control_items`, `next_client_message_id`, `strip_vn_diacritics`), AES/MD5/`signkey`, `ParamCipher v2`, `zpw_ws` WebSocket framing                |
| `session/{mod,domains,persistence,privacy,status}.rs`                                       | HTTP transport & failover (`api_get`/`api_post`), domain healing, `session.json`/`device.json`, Privacy Guard & redaction, `SessionStatus`                                                                                   |
| `auth/{mod,login,qr,trust/*}.rs`                                                            | Password/session info auth, QR1/QR2 flow, pure-Rust Curve25519/XEdDSA Trusted Device protocol                                                                                                                                |
| `social/{mod,friends,groups}.rs`                                                            | Friend roster/phone lookup/friendship requests, group profiles/history/name resolution                                                                                                                                       |
| `messaging/{mod,model,threads,send,polling,attachments,links,stickers,reactions,unread}.rs` | `ChatMessage`/`Thread` entities, `MessageIdentityIndex`, merge/dedup, text/RTF/quote/forward/recall, chunked file upload, OpenGraph link preview, stickers, `lp_chat` & `get-last-msgs` polling, reactions, seen/unread/mute |
| `sync/{mod,controls,zdb4,bin_net,v2_proto,history,phone,container}.rs`                      | Pure sync HTTP/WS controls, `/api/znoise`, RSA-2048 + `ZDB4.0` container crypto, `BinNet` TLV, Sync V2 Protobuf, cloud history & phone sync fetchers (zero `rusqlite`)                                                       |

## Non-Negotiables

- **Never commit secrets.** `session.json`, `*.db`, cookies, `zpw_sek`, `zpw_enk`, IMEI and capture dumps stay out of Git (`.gitignore`, `SESSION_SECURITY.md`).
- **No blocking work on the UI thread.** Network, disk, SQLite, and image decoding live inside `CoreEngine` (`crates/alolite-core/src/core/`); `apps/desktop` only invokes Tauri commands and renders `CoreEventDto`s.
- **All hosts live in `crates/alolite-core/src/protocol/endpoints.rs`.** Never hardcode a host elsewhere; declare HTTP command IDs in `crates/alolite-core/src/protocol/registry/` and invoke via `Session::api_get` / `Session::api_post`.
- **Single responsibility & pure protocol boundary.** Keep `rusqlite`, `MessageStore`, and `threads_cache.json` persistence in `crates/alolite-core/src/storage/` (`sync_ingest`, `thread_cache`) — never import `rusqlite` into `protocol/` and never import `Session`/HTTP into `storage/`. Reuse `wire.rs` helpers instead of duplicating JSON coercion, `group_` stripping, or control-envelope parsing.
- **Every outbound request goes through the standard path:** params → `ParamCipher v2` → `signkey` → session cookie attachment.
- **Comments in English, user-facing strings in Vietnamese.** See `docs/standards/code-comment-taxonomy.md`.
- **Do not silently change protocol behaviour.** Any change to params, signing, frame IDs or cipher flow requires evidence (capture, note in `re-notes/`) and a test or documented manual verification.
- **Missing or inaccurate RE documentation:** If reverse-engineering documentation (`re-notes/`, `docs/`) lacks information or deviates from live server behavior, engineers MUST use the `reverse-engineer-anything` skill and REA MCP tools to re-verify against actual application packages/endpoints rather than speculating.
- **Wire protocol docs live in `re-notes/`.** HTTP `commandId` catalog, wire envelopes, and WebSocket `cmd` registry → `re-notes/14-http-rest-endpoint-catalog.md`; directory index → `re-notes/README.md`. Do not duplicate upstream schema under `docs/`.
- **UI Icon Standard:** Always use embedded SVG/PNG icons (`lucide-react` vector icons and embedded PNG reaction/logo assets). NEVER use raw keyboard emojis (`⚠️`, `✅`, `❌`) in UI code.

## Issue Tracking

Issues and specifications live in **GitHub Issues** (`ngxccc/alolite`). Use the `gh` CLI. See `docs/standards/issue-tracker.md` and `.github/PULL_REQUEST_TEMPLATE.md`.

Every issue is atomic: one outcome, one boundary, one transaction or contract. Deferred bundles carry `needs-split` and are listed only in the split tracking issue. Query the register before planning or starting work on a bundle:

```bash
gh issue list --state open --label needs-split --limit 100 \
  --json number,title --jq '.[] | "#\(.number) \(.title)"'
```

## Engineering Standards

`docs/standards/` is the single source of truth, and reading the matching file is a gate before the first edit — not a suggestion. Do not reconstruct a rule from memory or from another repository.

| Change you are about to make               | Read first                                        |
| :----------------------------------------- | :------------------------------------------------ |
| An HTTP call, IPC command, or error shape  | `docs/standards/api-design-and-error-handling.md` |
| A UI page, component, token, or store      | `docs/standards/ui-and-frontend.md`               |
| A SQLite schema, query, or migration       | `docs/standards/database-and-migrations.md`       |
| A lock, channel, task, or retry            | `docs/standards/concurrency-and-locking.md`       |
| Secrets, crypto, or session handling       | `docs/standards/security-and-cryptography.md`     |
| A test, fixture, or benchmark              | `docs/standards/testing-and-fixtures.md`          |
| A comment or docstring                     | `docs/standards/code-comment-taxonomy.md`         |
| A branch, commit, or pull request          | `docs/standards/git-flow-and-pr-matrix.md`        |
| An issue, spec, or scope decision          | `docs/standards/issue-tracker.md`                 |
| A glossary term, domain model, or ADR      | `docs/standards/domain-docs.md`                   |
| A workflow, release, or packaging rule     | `docs/standards/ci-and-release.md`                |
| Reviewing a branch, diff, or PR            | `docs/standards/code-review.md`                   |
| Clean-room RE, IP, or licensing boundaries | `docs/standards/compliance-and-ip.md`             |

Protocol for every implementation task:

1. Map the planned edits to the table and read every matching file before writing code, tests, docs, or commit messages.
2. Open the response that accompanies the first edit with one line naming the read set, for example `Standards read: api-design-and-error-handling.md, testing-and-fixtures.md`.
3. When the diff grows into another area, read that area's standard before continuing instead of at the end.
4. When the mapping is unclear, read every file in `docs/standards/` plus `GLOSSARY.md` rather than guessing.
5. When code and a standard disagree, surface the conflict before changing either one.

# Code Review Standards

Repo-specific overrides and structural invariants for `/code-review` (complements `.agents/skills/code-review/references/layered-risk-checklist.md`).

## Tool & Noise Boundaries

- **Exclude generated artifacts from primary diff**: `apps/desktop/src/shared/types/ipc.generated.ts`, `Cargo.lock`, `bun.lock`, and `apps/desktop/dist/**`.
- **Skip tool-enforced checks**: leave Rust formatting/lints to `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings`, and TypeScript formatting/lints/types to `bun run check` (`tsc --noEmit`, `oxlint --type-aware`, `lint:tailwind`, `oxfmt --check`).
- **Spot-check generated contracts**: inspect `apps/desktop/src/shared/types/ipc.generated.ts` when Rust `specta` DTOs (`CoreEventDto`, Tauri commands) change.

## Repo-Specific Overrides

- **Pure protocol vs. storage boundary**: `crates/alolite-core/src/protocol/` MUST NEVER import `rusqlite`, and `crates/alolite-core/src/storage/` MUST NEVER import `Session` or perform HTTP requests (`ADR-0002`).
- **Single outbound HTTP & host SSOT**: all host URLs live in `crates/alolite-core/src/protocol/endpoints.rs`, command IDs live in `crates/alolite-core/src/protocol/registry/`, and outbound requests go through `Session::api_get` / `Session::api_post` (`ParamCipher v2` -> `signkey` -> cookie attachment). Reuse `wire.rs` helpers (`unwrap_data`, `pick_str`, `strip_group_prefix`) instead of ad-hoc JSON walks.
- **No blocking on the UI thread (INV-3)**: network, SQLite, crypto, and image decoding live inside `CoreEngine` (`crates/alolite-core/src/core/`); `crates/alolite-desktop` and `apps/desktop` only invoke Tauri commands and consume `CoreEventDto`s.
- **UI Icon & Primitives SSOT**: `apps/desktop` uses `lucide-react` and embedded PNG assets (never raw keyboard emojis) and composes controls from `apps/desktop/src/shared/ui/` (`@base-ui/react`, `ADR-0007`) and Zustand domain stores (`ADR-0006`).
- **No silent error swallowing**: `let _ = ...` is allowed only for best-effort cleanup with a Tier 2 comment (`docs/standards/api-design-and-error-handling.md`).

## Design & Structural Invariants

- **Model the Domain & Type Discipline**: Encode stateful logic and repeated shape assumptions in explicit state machines, tables/registries, or typed wire structs so illegal states are unrepresentable; parse external Zalo wire payloads at boundaries and keep domain logic pure.
- **Minimize Reader Load & Subtract First**: Remove dead weight before adding abstractions, collapse one-caller wrappers, and shrink mutable scope.
- **Clean Cutover (Migrate & Delete in One Wave)**: When introducing a new internal API or target architecture, migrate all callers and delete legacy APIs or throwaway compatibility shims in the same wave.

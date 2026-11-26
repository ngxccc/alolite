# UI & Frontend Standards

Governs `apps/desktop` (Tauri v2 + React 19 + TypeScript + Tailwind CSS v4 + `@base-ui/react`). Apply layers bottom-up (`UI-1` to `UI-5`).

## UI-1 — Design Tokens, Typography & Assets

- **3-Layer Zinc Tokens**: Define and consume CSS custom properties in `apps/desktop/src/app/styles/globals.css` (`--rail`, `--sidebar`, `--canvas`, shadcn core tokens, `--bubble-in`, `--bubble-out`, `--online`, `--warning`) mapped to the Zinc palette (`ADR-0007`). Never scatter hardcoded Zalo hex colors (`#005ae0`, `#e5f1ff`, `#1f344d`) across feature components.
- **Restrained Semantic Surfaces**: Use neutral surfaces at rest for destructive triggers (`Button variant="destructive"`) and status notices, reserving solid `bg-destructive` fills for hover/active destructive states. Keep UI transition durations `<= 220ms` with `prefers-reduced-motion` support.
- **Icons (Zero Raw Emojis)**: Import vector icons from `lucide-react` and use embedded PNG reaction/logo assets from `assets/`. **NEVER** use raw keyboard emojis (`⚠️`, `✅`, `❌`) in UI code (`AGENTS.md`). Use icons only where they add functional clarity; do not decorate repetitive static rows with redundant icons.
- **Offline `Inter` Typography & Numbers**: Bundle `@fontsource-variable/inter` locally for deterministic Linux `webkit2gtk-4.1` rendering (`ADR-0007`). Apply `tabular-nums` to counts, unread badges, file sizes, and timestamps; never make external Google Fonts or CDN font requests.

## UI-2 — Primitives & Feature Architecture

- **Shadcn Base UI Primitives SSOT**: Compose controls, inputs, labels, alerts, cards, badges, dialogs, dropdowns, context menus, switches, tabs, and tooltips from `apps/desktop/src/shared/ui/` (`@base-ui/react`, `ADR-0007`) rather than hand-rolling raw `<button>`, `<input>`, `<select>`, `<textarea>`, or modal markup in `features/`. Keep domain chat widgets in `apps/desktop/src/shared/ui/chat/` and `image-viewer-modal.tsx`.
- **Shadcn CLI-First Primitives**: Whenever a standard Shadcn Base UI primitive is missing from `apps/desktop/src/shared/ui/`, add it via `cd apps/desktop && bunx shadcn@latest add <component>` configured by `apps/desktop/components.json`. Never import `@radix-ui/*` or use `asChild`; compose triggers using Base UI's `render` prop and `data-checked`/`data-unchecked` selectors (`ADR-0007`).
- **Flat Surface Hierarchy (No Box-in-Box Nesting)**: Keep at most one surface container (`Card` / `border`) per visual region. Do not nest decorative inner bordered/tinted sub-boxes around static metadata or descriptions; reserve inner borders for interactive controls, discrete list rows with per-item actions, alert banners, and dialogs.
- **Feedback Channels**: Use inline status/alert banners for persistent connection, login, or validation errors that require user action; use ephemeral notifications strictly for standalone background confirmations whose result is not already visible in place.

## UI-3 — Tauri IPC, Stores & Trust Boundaries

- **Feature-Based Layout**: Organize `apps/desktop/src/` by `app/`, `features/<domain>/`, `shared/`, and `stores/` (`ADR-0003`). Cross-feature dependencies must remain acyclic; shared utilities live in `shared/`.
- **Strict IPC Boundary (INV-3)**: `apps/desktop` never performs direct HTTP requests, SQLite queries, file decryption, or blocking I/O. Invoke Tauri commands and consume typed `CoreEventDto` payloads (`apps/desktop/src/shared/types/ipc.generated.ts`).
- **Zustand Domain Stores & Selectors (`ADR-0006`)**: Keep domain state in dedicated Zustand stores (`apps/desktop/src/stores/`) with Immer updates where appropriate. Components MUST subscribe via fine-grained atomic selectors (`useThreadsStore((s) => s.activeThreadId)`), never destructuring whole stores at high-frequency render boundaries (chat list, message stream).
- **Virtualization**: Long message histories and thread lists use `@tanstack/react-virtual` so scrolling stays constant-time.

## UI-4 — Error Recovery & Accessibility

- **Sanitized Error Surfaces**: Show safe Vietnamese user-facing error messages; never dump raw cookies, `zpw_sek`, tokens, stack traces, or internal session fragments into the DOM.
- **Language & Accessibility**: All user-facing copy is Vietnamese; code, symbols, and comments are English (`docs/standards/code-comment-taxonomy.md`). Mark decorative icons `aria-hidden="true"`, provide `aria-label` on icon-only controls, and preserve keyboard navigation and `focus-visible` rings (verified in live `webkit2gtk-4.1` for Base UI portals/dialogs).

## UI-5 — Verification & Tests

- **Unit & Store Specs**: Write `*.test.ts` / `*.spec.ts` tests under `apps/desktop/` for pure state transitions, store selectors, and formatting helpers using `bun test`.
- **Required Gates**: Run `(cd apps/desktop && bun run check && bun test && bun run build)` cleanly before review (`oxfmt --check .`, `oxlint --type-aware`, `lint:tailwind`, `tsc --noEmit`, `bun test`, `vite build`).

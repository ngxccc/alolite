---
status: Accepted
date: 2026-11-25
deciders: ngxccc
---

# 7. shadcn/ui Base UI Primitives and Zinc Design System

## Context

`ADR-0003` established `apps/desktop/src/shared/ui/` as a hand-maintained collection of 14 components built on `@radix-ui/react-*`, `asChild` / `@radix-ui/react-slot`, and a 3-layer Zalo Slate / Zinc Dark token stylesheet (`apps/desktop/src/app/styles/globals.css`). Over time, three structural problems emerged:

- Feature slices (`settings/`, `contacts/`, `threads/`, `chat/`, `devtools/`, `navigation/`) still contain duplicated raw `<button>`, `<input>`, `<select>`, `<textarea>`, and ad-hoc modal/card markup instead of shared primitives.
- `apps/desktop/` has no `components.json`, preventing `shadcn` CLI upgrades or additions and causing drift across primitives.
- Radix `asChild` composition creates nested trigger wrappers (e.g., `TooltipTrigger asChild` wrapping `DropdownMenuPrimitive.Trigger asChild` wrapping `<button>` in `nav-rail.tsx`), while hardcoded Zalo hex colors (`#005ae0`, `#e5f1ff`, `#1f344d`, `#00378a`) are scattered across feature components.

- **Target Module**: `apps/desktop/components.json`, `apps/desktop/src/shared/ui/`, `apps/desktop/src/app/styles/globals.css`, `apps/desktop/src/features/`
- **Spec Reference**: `docs/design/shadcn-base-ui-zinc-migration.md`
- **Supersedes (partial)**: `docs/adr/0003-tauri-feature-ui-architecture.md` (Section 2.3 UI primitive library and color theme tokens)

## Decision

Migrate `apps/desktop/` UI primitives to a single-stack **shadcn/ui Base UI** (`@base-ui/react`) foundation and adopt the **Zinc minimalist design system**:

1. **Single-Stack Base UI (`@base-ui/react`)**: Initialize `apps/desktop/components.json` with a Base UI style preset, replace all 9 `@radix-ui/react-*` packages and `@radix-ui/react-slot` with `@base-ui/react`, and migrate every `asChild` call site to Base UI's `render` prop pattern.
2. **Demand-Driven Component Catalog**: Manage pure primitives in `apps/desktop/src/shared/ui/` via `components.json` and install only components that replace actual duplicated/raw UI in `features/` (`select`, `alert-dialog`, `card`, `textarea`, `label`), while keeping `src/shared/ui/chat/*` and `image-viewer-modal.tsx` as domain components. Unused registry components (Chart, Carousel, Calendar, Menubar, Pagination) are excluded.
3. **3-Layer Zinc Token Normalization**: Preserve the existing 3-layer CSS variable schema in `apps/desktop/src/app/styles/globals.css` (`--rail`/`--sidebar`/`--canvas`, shadcn core tokens, `--bubble-in`/`--bubble-out`/`--online`/`--warning`) mapped to the Zinc palette (`#FFFFFF`/`#18181B`/`#E4E4E7` in Light; `#09090B`/`#FAFAFA`/`#27272A` in Dark), without introducing Material 3 token names. Remove all hardcoded Zalo hex colors from feature components.
4. **Offline `Inter` Typography & Motion**: Bundle `@fontsource-variable/inter` locally for deterministic Linux `webkit2gtk-4.1` rendering with `tabular-nums`, and constrain UI transition durations (`<= 220ms`) with `prefers-reduced-motion` support.

## Positive Consequences

- Eliminates 9 separate `@radix-ui/react-*` dependencies and `Slot`/`asChild` indirection in favor of one unified `@base-ui/react` primitive model (`render` prop).
- Replaces duplicated raw HTML controls (`<select>`, `<button>`, `<input>`, ad-hoc confirm modals, settings cards) across `features/` with accessible, keyboard-navigable Base UI primitives.
- Unifies Light and Dark themes under a high-contrast Zinc palette with zero external font/CDN network calls.

## Negative Consequences / Risks

- **Breaking Selector & Composition Changes**: Base UI uses `data-checked`/`data-unchecked` instead of Radix's `data-[state=checked]`/`data-[state=unchecked]`, and `render` instead of `asChild`; missing any call site (such as `SwitchRow` in `primitives.tsx` or nested triggers in `nav-rail.tsx`) causes silent visual or focus regressions.
- **WebKitGTK Runtime Verification**: Portals, focus trapping, and positioning in `@base-ui/react` must be verified in the live `alolite-desktop` (`webkit2gtk-4.1`) binary, not only in `bun test`.
- **Bundle Footprint**: Bundling `@fontsource-variable/inter` adds ~300 KB to `apps/desktop/dist`.

## Explicit Tradeoffs

- **Single-Stack Big-Bang Primitive Swap vs Dual-Stack Incremental Migration**: Accept updating all `asChild` call sites in one coordinated migration rather than running `@radix-ui/*` and `@base-ui/react` simultaneously, avoiding competing portal/focus-trap stacks and double bundle weight.
- **Demand-Driven Registry Audit vs Full shadcn Dump**: Install only components with concrete feature demand (`Select`, `AlertDialog`, `Card`, `Textarea`, `Label`) instead of adding all ~50+ registry components, avoiding unused dependencies (`recharts`, `react-day-picker`, `embla-carousel-react`) and unnecessary `oxlint`/`tsc` surface.
- **Offline `@fontsource-variable/inter` vs System Font Fallback**: Accept ~300 KB local font asset overhead to guarantee consistent Vietnamese `Inter` + `tabular-nums` rendering across Linux distributions without CDN requests.

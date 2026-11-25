---
status: Accepted
date: 2026-11-25
deciders: ngxccc
---

# 6. Frontend Domain Stores and Selector Discipline

## Context

`ADR-0003` introduced `src/app/store/` (`appReducer.ts` + `useAppStore.ts`) as a single monolithic state store. A subsequent partial migration added four Zustand + Immer domain stores under `src/stores/` (`use-auth-store.ts`, `use-ui-store.ts`, `use-chat-store.ts`, `use-contacts-store.ts`), leaving the frontend in a split-brain state:

- `useAppStore.dispatch` still runs `appReducer` over the full `AppState` and then calls `syncDomainStores(next)` to overwrite all four domain stores via `setState(...)`, resetting fields such as `devDiagnostics`, `sendingMsgIds`, `syncProgress`, and `lastSyncStats` on every dispatch.
- Feature components subscribe to `useAppStore()` (and `useChatStore()`) without selectors, causing whole-tree re-renders on every `CORE_EVENT` and negating React Compiler optimizations.

- **Target Module**: `apps/desktop/src/stores/`, `apps/desktop/src/app/store/`, `apps/desktop/src/features/`
- **Supersedes (partial)**: `docs/adr/0003-tauri-feature-ui-architecture.md` (Section 2.3 state management)

## Decision

Consolidate frontend state management onto the four Zustand + Immer domain stores (`useAuthStore`, `useUIStore`, `useChatStore`, `useContactsStore`) as the sole source of truth:

1. Route `CoreEventDto` payloads directly to domain store actions instead of running a monolithic `appReducer`.
2. Remove `appReducer.ts`, `app-state.ts`, `useAppStore`, and `syncDomainStores`.
3. Enforce fine-grained Zustand selectors (`useChatStore((s) => s.selectedThreadId)`) across all feature components; whole-store subscriptions (`useStore()`) are prohibited.

## Positive Consequences

- Eliminates split-brain state overwrites and silent loss of domain-local fields (`sendingMsgIds`, `syncProgress`, `devDiagnostics`).
- Isolates re-renders to components subscribed to the specific slice changed by a `CORE_EVENT` or user action, allowing React Compiler memoization to work as intended.
- Aligns production code with the existing Immer store actions and `src/stores/stores.test.ts` test suite.

## Negative Consequences / Risks

- Requires touching all 8 feature slices (`auth`, `navigation`, `threads`, `chat`, `contacts`, `sync`, `settings`, `devtools`) to replace `useAppStore()` calls with domain-store selectors and actions.
- Cross-domain transitions (e.g., logout clearing chat + contacts state) must be coordinated explicitly in event handlers or a thin coordinator helper rather than a single reducer switch.

## Explicit Tradeoffs

- **Refactor Surface vs Render & State Integrity**: Accept a multi-file migration across `apps/desktop/src/features/` now instead of keeping a monolithic reducer or leaving `syncDomainStores` in place.
- **Explicit Cross-Store Coordination vs Single Global Reducer**: Accept explicit multi-store resets on lifecycle transitions (bootstrap, logout, account switch) in exchange for isolated per-domain updates on high-frequency chat/sync events.

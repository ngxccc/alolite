---
title: ZChat Minimalist Zinc Layout Polish
author: ngxccc
date: 2026-11-26
status: Approved
adr: docs/adr/0007-shadcn-base-ui-and-zinc-design-system.md
---

# ZChat Minimalist Zinc Layout Polish

## Goal

Apply the visual hierarchy, 1px border discipline, compact density, and 4-column proportions (`64px / 300px / 1fr / 300px`) from the ZChat minimalist Zinc mockup onto `apps/desktop/` while preserving 100% of existing `CoreEngine` features, `@base-ui/react` primitives (`ADR-0007`), offline assets (`@fontsource-variable/inter`, `lucide-react`, PNG reaction assets), and Light/Dark theme support.

## Scope & Decisions

1. **Token Adjustments (`apps/desktop/src/app/styles/globals.css`)**:
   - Light mode aligns with the crisp Zinc surface hierarchy: `--rail: #fafafa` (with `border-r`), `--rail-foreground: #09090b`, `--canvas: #ffffff`, `--bubble-in: #f4f4f5`, `--bubble-in-border: #e4e4e7`.
   - Dark mode (`[data-theme="dark"]`) retains the Zinc Dark palette (`#09090b` / `#18181b` / `#27272a`).
2. **4-Column Shell & Responsive `InfoDrawer`**:
   - Desktop layout: `64px` (`NavRail`) / `300px` (`ThreadsSidebar`) / `1fr` (`ChatArea`) / `300px` (`InfoDrawer`).
   - Narrow viewports (`< 1024px`): `InfoDrawer` renders as a right-aligned overlay drawer instead of squeezing `ChatArea` below usable width.
3. **Unsupported Mockup Elements Excluded**:
   - No `Công việc` nav tab, no `Gọi thoại` / `Gọi video` buttons, no fabricated group online count or creation date, and no raw keyboard emoji reactions (`👍`, `🔥`).
   - `+ Mới` in `ThreadsSidebar` opens the existing `PhoneLookupModal`.
4. **Component Refinements**:
   - `NavRail`: light-compatible `bg-rail` + `border-r`, `h-10 w-10 rounded-md` buttons, `bg-primary` active state.
   - `ThreadsSidebar`: `"Tin nhắn"` header + `Phân loại` filter menu + `"Mới"` button, `Ctrl+K`/`⌘K` search shortcut (`Esc` to clear/blur), segmented `Tất cả | Chưa đọc (N)` tabs.
   - `ChatHeader` & `PinnedBar`: `64px` header with pill member-count badge and single-line `PinnedBar`.
   - `MessageBubble` & `Composer`: `rounded-lg` 1px-bordered bubbles, `w-0.5 bg-primary` quote/reply accent bar, preserving PNG reactions, sticker picker, link preview, and file upload.
   - `InfoDrawer`: `bg-muted/20 border-l`, 1px `border-b` dividers (replacing 8px `SectionBand`), and `TabsList` (`Ảnh | File | Link`) for shared media.

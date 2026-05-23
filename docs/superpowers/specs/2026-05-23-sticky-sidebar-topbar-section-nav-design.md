# Sticky Sidebar & TopBar Section Nav

**Date:** 2026-05-23
**Status:** Approved

## Problem

The two-pane dashboard (introduced in `b7df7e8`) has two visual issues:

1. **Sidebar scrolls away on desktop.** `Sidebar.tsx` declares `sticky top-11` but is then overridden by `lg:static`, so on large screens the left filter panel scrolls out of view with the rest of the page.
2. **Section nav (用量/订阅/请求) lives in its own row.** It sits below `GlanceBand` as a separate sticky bar, adding visual weight and competing with the TopBar for vertical space. The user wants these three switches always visible alongside the title for a more consistent header.

## Goals

- The left filter panel stays pinned during vertical scroll on desktop.
- The three section switches (`用量`, `订阅`, `请求`) are visible inside the TopBar regardless of scroll position.
- Existing scroll-spy behavior (IntersectionObserver updates active section) and click-to-scroll behavior are preserved.

## Non-goals

- No changes to filter content, KPI strip, or section bodies.
- No styling of the sidebar beyond removing `lg:static`.
- No mobile layout overhaul — keep the existing `max-lg:fixed` drawer behavior.

## Design

### Component changes

| File | Change |
|---|---|
| `frontend/src/components/TopBar.tsx` | Accept `activeSection: SectionId` and `onSectionSelect: (id: SectionId) => void`. Render the three switches inside the TopBar, absolutely positioned/centered with `left-1/2 -translate-x-1/2` so the centered group stays centered independent of title or right-cluster width. |
| `frontend/src/components/SectionNav.tsx` | Delete file. The `SectionId` type moves into `TopBar.tsx` and is re-exported from there. |
| `frontend/src/components/Sidebar.tsx` | Remove the `lg:static` class on the `<aside>`. Keep `sticky top-11 h-[calc(100vh-2.75rem)]` so desktop scroll keeps the panel pinned. The flex parent in `App.tsx` needs `items-start` so the flex item respects its own height instead of stretching. |
| `frontend/src/App.tsx` | Drop the `<SectionNav>` JSX and its import. Pass `activeSection` and `handleSectionSelect` to `<TopBar>`. Add `items-start` to the `flex` container wrapping Sidebar + main. |

### State

`activeSection` state remains in `App.tsx`. The `IntersectionObserver` in `App.tsx` continues to update `activeSection` based on scroll position. The click handler `handleSectionSelect` is unchanged — it just gets passed to `TopBar` instead of `SectionNav`.

### Layout (desktop)

```
┌──────────────────────────────────────────────────────────────┐
│ ⚡ Title          [用量][订阅][请求]      Updated 14:32  ↻  │  ← sticky TopBar (h-11, z-30)
├──────────┬───────────────────────────────────────────────────┤
│ Sidebar  │ GlanceBand                                        │
│ (sticky) │ UsageSection                                      │
│  filters │ QuotasSection                                     │
│  models  │ RequestsSection                                   │
│          │ footer                                            │
└──────────┴───────────────────────────────────────────────────┘
```

On scroll: TopBar stays pinned (already does), Sidebar stays pinned (new), main scrolls underneath.

### Edge cases

- **Narrow viewport**: the centered absolute-positioned switch group has fixed width (~180px for three short Chinese labels). The title gets `truncate min-w-0` so it shrinks instead of pushing the switches off-center.
- **Mobile (`max-lg`)**: the menu button stays on the left. Switches still render centered. The right cluster shrinks (timestamp may hide via `hidden sm:inline`).
- **SettingsDrawer**: replaces Sidebar in the same flex slot, so the same sticky rules apply to it via its own classes (no change needed if it already follows Sidebar's pattern; verify during implementation).

### Testing

- `cd frontend && npm run lint`
- `cd frontend && npx tsc --noEmit` (typecheck)
- `cd backend && cargo fmt --check && cargo clippy -- -D warnings` (CI parity)
- Manual: start `npm run dev`, scroll the dashboard, confirm sidebar stays pinned, switches stay visible, clicking a switch scrolls to the section, scrolling between sections updates the active switch.

## Risks

- The `absolute` centered switch group could overlap the title on very narrow desktop widths (~600–800px). Mitigation: `truncate` on title; switches are short enough (~180px) that overlap requires extremely narrow viewports where mobile layout would already apply.
- Removing `lg:static` could expose a previously-hidden bug if any ancestor has `overflow: hidden` (which breaks sticky). Verify by inspecting the chain from `<aside>` to `<body>` during implementation.

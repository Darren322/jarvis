/**
 * Which Panel the UI shows, and what it shows in it. Jarvis decides (jarvis-ui
 * D2, D8); this only follows its messages.
 */

import type { BuiltPanel, NamedPanel, PanelMessage, Stats } from './protocol';

/**
 * No Panel, the stats Dashboard with its latest reading (`null` until the first
 * one arrives), the camera (its picture comes from `/camera`, not from here),
 * or a built Panel with its parts and the latest reading its gauges show.
 */
export type PanelView =
  | { readonly kind: 'none' }
  | { readonly kind: 'stats'; readonly reading: Stats | null }
  | { readonly kind: 'camera' }
  | { readonly kind: 'built'; readonly panel: BuiltPanel; readonly reading: Stats | null };

export const NO_PANEL: PanelView = { kind: 'none' };

/** A Panel just opened: a Dashboard with no reading yet, or the camera. */
function opened(panel: NamedPanel): PanelView {
  switch (panel) {
    case 'stats':
      return { kind: 'stats', reading: null };
    case 'camera':
      return { kind: 'camera' };
  }
}

/**
 * The Panel after one message. ShowPanel opens with no reading, so a reopened
 * Dashboard never shows an old one; showing the Panel already open changes
 * nothing. ShowBuiltPanel always opens its own parts, with no reading.
 * HidePanel closes it. A reading counts only while the Dashboard or a built
 * Panel shows. Returns `view` itself when nothing changes.
 */
export function applyPanelMessage(view: PanelView, message: PanelMessage): PanelView {
  switch (message.type) {
    case 'show_panel':
      return view.kind === message.panel ? view : opened(message.panel);
    case 'show_built_panel':
      return { kind: 'built', panel: message.panel, reading: null };
    case 'hide_panel':
      return NO_PANEL;
    case 'stats':
      switch (view.kind) {
        case 'stats':
          return { kind: 'stats', reading: message.stats };
        case 'built':
          return { ...view, reading: message.stats };
        case 'none':
        case 'camera':
          return view;
      }
  }
}

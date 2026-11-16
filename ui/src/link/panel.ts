/**
 * Which Panel the UI shows, and what it shows in it. Jarvis decides (jarvis-ui
 * D2, D8); this only follows its messages.
 */

import type { PanelMessage, Stats } from './protocol';

/** No Panel, or the stats Dashboard with its latest reading (`null` until the first one arrives). */
export type PanelView = { readonly kind: 'none' } | { readonly kind: 'stats'; readonly reading: Stats | null };

export const NO_PANEL: PanelView = { kind: 'none' };

/**
 * The Panel after one message. ShowPanel opens with no reading, so a reopened
 * Dashboard never shows an old one; showing the Panel already open changes
 * nothing. HidePanel closes it. A reading counts only while the Dashboard
 * shows. Returns `view` itself when nothing changes.
 */
export function applyPanelMessage(view: PanelView, message: PanelMessage): PanelView {
  switch (message.type) {
    case 'show_panel':
      return view.kind === message.panel ? view : { kind: message.panel, reading: null };
    case 'hide_panel':
      return NO_PANEL;
    case 'stats':
      return view.kind === 'stats' ? { kind: 'stats', reading: message.stats } : view;
  }
}

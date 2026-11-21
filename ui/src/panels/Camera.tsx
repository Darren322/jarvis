/**
 * The camera Panel: Jarvis's `/camera` route in a plain `<img>`. Chromium shows
 * the MJPEG stream itself, frame after frame (jarvis-ui D9, ticket 11). The
 * picture never reaches React: mounting the image opens its request and
 * unmounting it (the Panel closing) ends it.
 */
import { useState } from 'react';

export interface CameraProps {
  /** The `/camera` URL, from the same host and port as the WebSocket. */
  readonly url: string;
}

/** Showings of the camera Panel since the page loaded. */
let showings = 0;

/**
 * `url` marked as one showing of the Panel. Chromium keeps the last picture of
 * an `<img>` URL it has already loaded and shows that still frame again instead
 * of asking anew (seen on the Pi, 2026-11-21: shown, closed and shown again, the
 * picture froze). A URL of its own for every showing makes each one a new
 * request. Jarvis ignores the query.
 */
export function showingUrl(url: string, showing: number, now: number): string {
  const marked = new URL(url);
  marked.searchParams.set('showing', `${now}-${showing}`);
  return marked.href;
}

export function Camera({ url }: CameraProps) {
  // Fixed for the life of this showing: a render must not restart the stream.
  const [source] = useState(() => {
    showings += 1;
    return showingUrl(url, showings, Date.now());
  });
  return <img className="camera-picture" src={source} alt="Camera" draggable={false} />;
}

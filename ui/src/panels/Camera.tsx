/**
 * The camera Panel: Jarvis's `/camera` route in a plain `<img>`. Chromium shows
 * the MJPEG stream itself, frame after frame (jarvis-ui D9, ticket 11). The
 * picture never reaches React: mounting the image opens its request and
 * unmounting it (the Panel closing) ends it.
 */

export interface CameraProps {
  /** The `/camera` URL, from the same host and port as the WebSocket. */
  readonly url: string;
}

export function Camera({ url }: CameraProps) {
  return <img className="camera-picture" src={url} alt="Camera" draggable={false} />;
}

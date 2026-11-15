import { useEffect, useState } from 'react';
import { JarvisLink } from '../link/link';
import { Orb, type OrbSize } from './Orb';
import type { CreateOrbRenderer } from './renderer';

const TOP_LEFT = { left: 0, top: 0 } as const;

function windowSize(): OrbSize {
  return { width: window.innerWidth, height: window.innerHeight };
}

/** The window's size in CSS pixels, updated on resize (rare: the kiosk is fixed at 800x480). */
function useWindowSize(): OrbSize {
  const [size, setSize] = useState(windowSize);
  useEffect(() => {
    const onResize = () => setSize(windowSize());
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);
  return size;
}

export interface JarvisOrbProps {
  readonly createRenderer: CreateOrbRenderer;
}

/** The full-screen Orb, connected to Jarvis over the WebSocket. */
export function JarvisOrb({ createRenderer }: JarvisOrbProps) {
  // Created once; connecting happens in the effect, so a render never opens a socket.
  const [link] = useState(() => new JarvisLink());
  useEffect(() => {
    link.start();
    return () => link.stop();
  }, [link]);
  const size = useWindowSize();
  return <Orb source={link} onTap={link.tap} size={size} position={TOP_LEFT} createRenderer={createRenderer} />;
}

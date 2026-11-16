import { useEffect, useState } from 'react';
import { JarvisLink } from './link/link';
import { createOrbRenderer } from './orb/drawing';
import { Screen } from './panels/Screen';

/** The whole screen: the Orb and its Panels, sharing one connection to Jarvis. */
export function App() {
  // Created once; connecting happens in the effect, so a render never opens a socket.
  const [link] = useState(() => new JarvisLink());
  useEffect(() => {
    link.start();
    return () => link.stop();
  }, [link]);
  return (
    <main aria-label="Jarvis">
      <Screen source={link} onTap={link.tap} createRenderer={createOrbRenderer} />
    </main>
  );
}

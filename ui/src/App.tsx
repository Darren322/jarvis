import { createOrbRenderer } from './orb/drawing';
import { JarvisOrb } from './orb/JarvisOrb';

/** The whole screen: the Orb, connected to Jarvis. Panels arrive in later tickets. */
export function App() {
  return (
    <main aria-label="Jarvis">
      <JarvisOrb createRenderer={createOrbRenderer} />
    </main>
  );
}

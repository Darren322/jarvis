import { useEffect, useState, useSyncExternalStore } from 'react';
import { CAMERA_URL, type OrbSource, type PanelSource } from '../link/link';
import type { PanelName } from '../link/protocol';
import { type FrameScheduler, Orb, type OrbSize } from '../orb/Orb';
import type { CreateOrbRenderer } from '../orb/renderer';
import { Built } from './Built';
import { Camera } from './Camera';
import { Dashboard } from './Dashboard';
import { orbPlacement, PANEL_INSETS } from './layout';
import { PANEL_COLOR_STYLE } from './palette';
import './panels.css';

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

/** The stats Dashboard, fed from the source. Only this re-renders when a reading arrives. */
function LiveDashboard({ source }: { readonly source: PanelSource }) {
  const reading = useSyncExternalStore(source.subscribePanel, () => source.stats);
  return <Dashboard reading={reading} />;
}

/**
 * A built Panel, fed from the source: its parts, and the latest reading for its
 * gauges. Only this re-renders when a reading arrives.
 */
function LiveBuilt({ source }: { readonly source: PanelSource }) {
  const panel = useSyncExternalStore(source.subscribePanel, () => source.builtPanel ?? null);
  const reading = useSyncExternalStore(source.subscribePanel, () => source.stats);
  return panel === null ? null : <Built panel={panel} reading={reading} />;
}

/** What the Panel shows inside its frame. */
function PanelContent({ panel, source }: { readonly panel: PanelName; readonly source: PanelSource }) {
  switch (panel) {
    case 'stats':
      return <LiveDashboard source={source} />;
    case 'camera':
      return <Camera url={source.cameraUrl ?? CAMERA_URL} />;
    case 'built':
      return <LiveBuilt source={source} />;
  }
}

/** The Panel's frame: the same place and border for every Panel. A tap inside it does nothing. */
function PanelFrame({ panel, source }: { readonly panel: PanelName; readonly source: PanelSource }) {
  return (
    <section
      className={`panel panel-${panel}`}
      aria-label={PANEL_LABELS[panel]}
      style={{ ...PANEL_INSETS, ...PANEL_COLOR_STYLE }}
    >
      <PanelContent panel={panel} source={source} />
    </section>
  );
}

const PANEL_LABELS: Readonly<Record<PanelName, string>> = { stats: 'Stats', camera: 'Camera', built: 'Built panel' };

export interface ScreenProps {
  /** Where the Orb's state and the Panel come from: the link, or a fixture in the preview. */
  readonly source: OrbSource & PanelSource;
  /** Called on a tap on the Orb. */
  readonly onTap: () => void;
  readonly createRenderer: CreateOrbRenderer;
  /** The Orb's frame scheduler; default: the browser's display frames. */
  readonly frames?: FrameScheduler;
}

/**
 * The whole screen: the Orb, and the Panel Jarvis asked for (jarvis-ui D8).
 *
 * While a Panel shows, the Orb sits in a corner (`orbPlacement`) and a layer
 * behind the Panel catches taps outside it, which close the Panel. The Orb is
 * drawn last, so it stays on top and a tap on it is still a Tap. This
 * re-renders only when the shown Panel or the window changes: state and level
 * messages never reach React, and readings re-render only the Dashboard or the
 * built Panel.
 */
export function Screen({ source, onTap, createRenderer, frames }: ScreenProps) {
  const panel = useSyncExternalStore(source.subscribePanel, () => source.shownPanel);
  const { size, position, round } = orbPlacement(useWindowSize(), panel);
  return (
    <>
      {panel === null ? null : (
        <>
          <div className="panel-outside" aria-hidden="true" onPointerDown={source.closePanel} />
          <PanelFrame panel={panel} source={source} />
        </>
      )}
      <Orb
        source={source}
        onTap={onTap}
        size={size}
        position={position}
        round={round}
        createRenderer={createRenderer}
        frames={frames}
      />
    </>
  );
}

import { useEffect, useState, useSyncExternalStore } from 'react';
import { CAMERA_URL, type OrbSource, type PanelSource, type TimerSource } from '../link/link';
import type { PanelName } from '../link/protocol';
import { type FrameScheduler, Orb, type OrbSize } from '../orb/Orb';
import type { CreateOrbRenderer } from '../orb/renderer';
import { Built } from './Built';
import { Camera } from './Camera';
import { Dashboard } from './Dashboard';
import { orbPlacement, panelInsets } from './layout';
import { PANEL_COLOR_STYLE } from './palette';
import { Timers } from './Timers';
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

/**
 * The Panel's frame: the same place and border for every Panel, narrowed
 * while the timer strip has rows (`isBesideTimers`). A tap inside it does nothing.
 */
function PanelFrame({
  panel,
  source,
  isBesideTimers,
}: {
  readonly panel: PanelName;
  readonly source: PanelSource;
  readonly isBesideTimers: boolean;
}) {
  return (
    <section
      className={`panel panel-${panel}${isBesideTimers ? ' panel-beside-timers' : ''}`}
      aria-label={PANEL_LABELS[panel]}
      style={{ ...panelInsets(isBesideTimers), ...PANEL_COLOR_STYLE }}
    >
      <PanelContent panel={panel} source={source} />
    </section>
  );
}

/** The timer strip, fed from the source. Only this re-renders when a `timers` message arrives or its clock ticks. */
function LiveTimers({ source }: { readonly source: TimerSource }) {
  const shown = useSyncExternalStore(source.subscribeTimers, () => source.shownTimers);
  return (
    <Timers
      shown={shown}
      onConfirm={source.confirmTimer}
      onReject={source.rejectTimer}
      onCancel={source.cancelTimer}
    />
  );
}

const PANEL_LABELS: Readonly<Record<PanelName, string>> = { stats: 'Stats', camera: 'Camera', built: 'Built panel' };

export interface ScreenProps {
  /** Where the Orb's state, the Panel and the timers come from: the link, or a fixture in the preview. */
  readonly source: OrbSource & PanelSource & TimerSource;
  /** Called on a tap on the Orb. */
  readonly onTap: () => void;
  readonly createRenderer: CreateOrbRenderer;
  /** The Orb's frame scheduler; default: the browser's display frames. */
  readonly frames?: FrameScheduler;
}

/**
 * The whole screen: the Orb, the Panel Jarvis asked for (jarvis-ui D8) and
 * the timer strip (timers D15).
 *
 * While a Panel shows, the Orb sits in a corner (`orbPlacement`) and a layer
 * behind the Panel catches taps outside it, which close the Panel. The Orb is
 * drawn last, so it stays on top and a tap on it is still a Tap. This
 * re-renders only when the shown Panel or the window changes: state and level
 * messages never reach React, and readings re-render only the Dashboard or the
 * built Panel.
 *
 * The timer strip is drawn last, at the top right: above the full-size Orb and
 * the layer that closes a Panel, so a tap on it reaches only the strip. While
 * it has rows the Panel narrows to leave it room (`panelInsets`); this
 * re-renders only when the strip gains its first row or loses its last.
 */
export function Screen({ source, onTap, createRenderer, frames }: ScreenProps) {
  const panel = useSyncExternalStore(source.subscribePanel, () => source.shownPanel);
  const hasTimers = useSyncExternalStore(source.subscribeTimers, () => source.shownTimers.timers.length > 0);
  const { size, position, round } = orbPlacement(useWindowSize(), panel);
  return (
    <>
      {panel === null ? null : (
        <>
          <div className="panel-outside" aria-hidden="true" onPointerDown={source.closePanel} />
          <PanelFrame panel={panel} source={source} isBesideTimers={hasTimers} />
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
      <LiveTimers source={source} />
    </>
  );
}

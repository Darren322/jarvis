import type { CSSProperties } from 'react';
import { type GaugeSource, MAX_CPU_PERCENT, type ServiceHealth, type Stats } from '../link/protocol';
import {
  formatCelsius,
  formatHealth,
  formatPercent,
  formatRam,
  formatStorage,
  meterFraction,
  type Readout,
  storageUsedBytes,
  TEMPERATURE_METER_FULL_C,
  UNAVAILABLE,
  WAITING,
  WAITING_READOUT,
} from './format';

export interface DashboardProps {
  /** The latest Stats reading, or `null` before the first one: every value then shows `WAITING`. */
  readonly reading: Stats | null;
}

/** A section title with a thin rule running to a note on the right, if any. */
export function Heading({ title, note }: { readonly title: string; readonly note?: string }) {
  return (
    <h2 className="panel-heading">
      <span className="panel-heading-title">{title}</span>
      <span className="panel-rule" aria-hidden="true" />
      {note === undefined ? null : <span className="panel-heading-note">{note}</span>}
    </h2>
  );
}

interface MetricProps {
  readonly label: string;
  readonly readout: Readout;
  /** How full the meter under the value is, 0 to 1. */
  readonly fraction: number;
  /** Drawn smaller, so a "used / total GB" value fits one column. */
  readonly compact?: boolean;
}

/** A value with no number in it (a missing reading, or none yet) is drawn small and dim, so it never reads as a figure. */
function isWord(readout: Readout): boolean {
  return readout.figure === UNAVAILABLE || readout.figure === WAITING;
}

/** One labelled value with a thin meter under it. The meter repeats the value, so screen readers skip it. */
function Metric({ label, readout, fraction, compact = false }: MetricProps) {
  return (
    <div className={compact ? 'dashboard-metric dashboard-metric-compact' : 'dashboard-metric'}>
      <dt className="panel-label">{label}</dt>
      <dd className="dashboard-value">
        <span className={isWord(readout) ? 'dashboard-word' : 'dashboard-figure'}>
          {readout.figure}
        </span>
        {readout.unit === '' ? null : <span className="dashboard-unit">{readout.unit}</span>}
      </dd>
      <dd className="dashboard-meter" aria-hidden="true">
        <span style={{ '--fill': fraction } as CSSProperties} />
      </dd>
    </div>
  );
}

/** A service as a marker and a word: a filled dot for up, an amber ring for down, a grey ring while waiting. */
function Service({ name, health }: { readonly name: string; readonly health: ServiceHealth | null }) {
  return (
    <div className={`dashboard-service dashboard-service-${health ?? 'waiting'}`}>
      <dt className="panel-label">
        <span className="dashboard-marker" aria-hidden="true" />
        {name}
      </dt>
      <dd className="dashboard-health">{health === null ? WAITING : formatHealth(health)}</dd>
    </div>
  );
}

export interface GaugeProps {
  readonly source: GaugeSource;
  /** The latest Stats reading, or `null` before the first one. */
  readonly reading: Stats | null;
}

/**
 * One figure from a Stats reading, as the Dashboard draws it: a labelled value
 * with a meter, or a service's marker and word. The Dashboard and a built
 * Panel's gauges both draw through this, so the two never disagree. Belongs in
 * a `<dl>`.
 */
export function Gauge({ source, reading }: GaugeProps) {
  switch (source) {
    case 'cpu':
      return (
        <Metric
          label="CPU"
          readout={reading === null ? WAITING_READOUT : formatPercent(reading.cpuPercent)}
          fraction={reading === null ? 0 : meterFraction(reading.cpuPercent, MAX_CPU_PERCENT)}
        />
      );
    case 'temperature': {
      const temperatureC = reading?.temperatureC ?? null;
      return (
        <Metric
          label="Temperature"
          readout={reading === null ? WAITING_READOUT : formatCelsius(temperatureC)}
          fraction={temperatureC === null ? 0 : meterFraction(temperatureC, TEMPERATURE_METER_FULL_C)}
        />
      );
    }
    case 'ram':
      return (
        <Metric
          label="Memory"
          compact
          readout={reading === null ? WAITING_READOUT : formatRam(reading.ramUsedBytes, reading.ramTotalBytes)}
          fraction={reading === null ? 0 : meterFraction(reading.ramUsedBytes, reading.ramTotalBytes)}
        />
      );
    case 'storage': {
      const storage = reading?.storage ?? null;
      return (
        <Metric
          label="Storage"
          compact
          readout={reading === null ? WAITING_READOUT : formatStorage(storage)}
          fraction={storage === null ? 0 : meterFraction(storageUsedBytes(storage), storage.totalBytes)}
        />
      );
    }
    case 'llama':
      return <Service name="llama" health={reading?.llama ?? null} />;
    case 'bge':
      return <Service name="BGE" health={reading?.bge ?? null} />;
  }
}

/**
 * The stats Dashboard (jarvis-ui D7): CPU, temperature, RAM and storage of the
 * 8 GB Pi, and whether llama and BGE on the 16 GB Pi are up. It draws what it
 * is given and nothing else: no timer and no polling. While it shows, Jarvis
 * sends a new reading about once a second.
 */
export function Dashboard({ reading }: DashboardProps) {
  return (
    <>
      <div className="dashboard-group" aria-busy={reading === null}>
        <Heading title="System" note="8 GB Pi" />
        <dl className="dashboard-metrics">
          <Gauge source="cpu" reading={reading} />
          <Gauge source="temperature" reading={reading} />
          <Gauge source="ram" reading={reading} />
          <Gauge source="storage" reading={reading} />
        </dl>
      </div>
      <div className="dashboard-group" aria-busy={reading === null}>
        <Heading title="Models" note="16 GB Pi" />
        <dl className="dashboard-services">
          <Gauge source="llama" reading={reading} />
          <Gauge source="bge" reading={reading} />
        </dl>
      </div>
    </>
  );
}

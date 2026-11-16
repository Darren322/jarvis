import type { CSSProperties } from 'react';
import { MAX_CPU_PERCENT, type ServiceHealth, type Stats } from '../link/protocol';
import {
  formatCelsius,
  formatHealth,
  formatPercent,
  formatRam,
  meterFraction,
  type Readout,
  TEMPERATURE_METER_FULL_C,
  UNAVAILABLE,
  WAITING,
  WAITING_READOUT,
} from './format';

export interface DashboardProps {
  /** The latest Stats reading, or `null` before the first one: every value then shows `WAITING`. */
  readonly reading: Stats | null;
}

/** A section title with a thin rule running to a note on the right. */
function Heading({ title, note }: { readonly title: string; readonly note: string }) {
  return (
    <h2 className="panel-heading">
      <span>{title}</span>
      <span className="panel-rule" aria-hidden="true" />
      <span className="panel-heading-note">{note}</span>
    </h2>
  );
}

interface MetricProps {
  readonly label: string;
  readonly readout: Readout;
  /** How full the meter under the value is, 0 to 1. */
  readonly fraction: number;
  /** Spans both columns, for the widest value. */
  readonly wide?: boolean;
}

/** A value with no number in it (a missing reading, or none yet) is drawn small and dim, so it never reads as a figure. */
function isWord(readout: Readout): boolean {
  return readout.figure === UNAVAILABLE || readout.figure === WAITING;
}

/** One labelled value with a thin meter under it. The meter repeats the value, so screen readers skip it. */
function Metric({ label, readout, fraction, wide = false }: MetricProps) {
  return (
    <div className={wide ? 'dashboard-metric dashboard-metric-wide' : 'dashboard-metric'}>
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

/**
 * The stats Dashboard (jarvis-ui D7): CPU, temperature and RAM of the 8 GB Pi,
 * and whether llama and BGE on the 16 GB Pi are up. It draws what it is given
 * and nothing else: no timer and no polling. While it shows, Jarvis sends a
 * new reading about once a second.
 */
export function Dashboard({ reading }: DashboardProps) {
  const temperatureC = reading?.temperatureC ?? null;
  return (
    <>
      <div className="dashboard-group" aria-busy={reading === null}>
        <Heading title="System" note="8 GB Pi" />
        <dl className="dashboard-metrics">
          <Metric
            label="CPU"
            readout={reading === null ? WAITING_READOUT : formatPercent(reading.cpuPercent)}
            fraction={reading === null ? 0 : meterFraction(reading.cpuPercent, MAX_CPU_PERCENT)}
          />
          <Metric
            label="Temperature"
            readout={reading === null ? WAITING_READOUT : formatCelsius(temperatureC)}
            fraction={temperatureC === null ? 0 : meterFraction(temperatureC, TEMPERATURE_METER_FULL_C)}
          />
          <Metric
            label="Memory"
            wide
            readout={reading === null ? WAITING_READOUT : formatRam(reading.ramUsedBytes, reading.ramTotalBytes)}
            fraction={reading === null ? 0 : meterFraction(reading.ramUsedBytes, reading.ramTotalBytes)}
          />
        </dl>
      </div>
      <div className="dashboard-group" aria-busy={reading === null}>
        <Heading title="Models" note="16 GB Pi" />
        <dl className="dashboard-services">
          <Service name="llama" health={reading?.llama ?? null} />
          <Service name="BGE" health={reading?.bge ?? null} />
        </dl>
      </div>
    </>
  );
}

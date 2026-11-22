import { useEffect, useState } from 'react';
import type { ShownTimers } from '../link/link';
import type { Timer } from '../link/protocol';
import { COUNTDOWN_TICK_MS, formatCountdown, timerName } from './countdown';
import { TIMER_STRIP_BOX } from './layout';
import { PANEL_COLOR_STYLE } from './palette';
import './timers.css';

export interface TimersProps {
  /** The last `timers` message and when it arrived. */
  readonly shown: ShownTimers;
  /** Set on a pending row. */
  readonly onConfirm: (id: number) => void;
  /** Don't on a pending row. */
  readonly onReject: (id: number) => void;
  /** Cancel, after the question, on a running row; or a tap on an ended row. */
  readonly onCancel: (id: number) => void;
}

/**
 * The UI's clock while a row counts down: one interval for the whole strip,
 * reading `Date.now()` every `COUNTDOWN_TICK_MS`, stopped when no row runs and
 * on unmount. 0 until its first tick; the caller takes the later of this and
 * the message's arrival, so a new message is right before the first tick.
 */
function useClock(isCounting: boolean): number {
  const [now, setNow] = useState(0);
  useEffect(() => {
    if (!isCounting) {
      return;
    }
    const id = setInterval(() => setNow(Date.now()), COUNTDOWN_TICK_MS);
    return () => clearInterval(id);
  }, [isCounting]);
  return now;
}

interface RowProps {
  readonly timer: Timer;
  readonly name: string;
}

/** A timer Jarvis proposed: Set or Don't, which Jarvis checks against its own 30-second window (timers D4). */
function PendingRow({ timer, name, onConfirm, onReject }: RowProps & Pick<TimersProps, 'onConfirm' | 'onReject'>) {
  return (
    <li className="timer timer-pending">
      <span className="timer-name" title={name}>
        {name}
      </span>
      <div className="timer-actions">
        <button
          type="button"
          className="timer-button timer-button-yes"
          aria-label={`Set '${name}'`}
          onClick={() => onConfirm(timer.id)}
        >
          Set
        </button>
        <button
          type="button"
          className="timer-button"
          aria-label={`Don't set '${name}'`}
          onClick={() => onReject(timer.id)}
        >
          Don't
        </button>
      </div>
    </li>
  );
}

/** "Cancel '<label>'?" in place of a running row: Cancel sends `cancel_timer`; Keep only closes the question (timers D11). */
function CancelQuestion({
  timer,
  name,
  onCancel,
  onClose,
}: RowProps & Pick<TimersProps, 'onCancel'> & { readonly onClose: () => void }) {
  const question = `Cancel '${name}'?`;
  return (
    <li className="timer timer-question">
      <div role="group" aria-label={question}>
        <p className="timer-question-text" title={question}>
          {question}
        </p>
        <div className="timer-actions">
          <button
            type="button"
            className="timer-button timer-button-alert"
            onClick={() => {
              onClose();
              onCancel(timer.id);
            }}
          >
            Cancel
          </button>
          {/* Keep is the safe answer, so it takes the focus. */}
          <button type="button" className="timer-button" autoFocus onClick={onClose}>
            Keep
          </button>
        </div>
      </div>
    </li>
  );
}

/** A running or ended row: the whole row is one button. */
function TapRow({ timer, name, value, onTap }: RowProps & { readonly value: string; readonly onTap: () => void }) {
  return (
    <li className={`timer timer-${timer.state}`}>
      <button type="button" className="timer-row" onClick={onTap}>
        <span className="timer-name" title={name}>
          {name}
        </span>
        {/* A space, so the button's name reads "Time to sleep 25:00"; the flex layout draws none. */}{' '}
        <span className="timer-value">{value}</span>
      </button>
    </li>
  );
}

/**
 * The timer strip (timers D15): one row per timer in the last `timers`
 * message, in its order, at the top right. Nothing at all with no timers. It
 * decides nothing (jarvis-ui D2): it shows what Jarvis sent, counts down with
 * the UI's own clock, and sends ids back.
 *
 * - pending: the label with Set and Don't;
 * - running: the label and the time left; a tap asks "Cancel '<label>'?";
 * - ended: the label and "Ended", flashing; a tap clears it (D17), no question.
 */
export function Timers({ shown, onConfirm, onReject, onCancel }: TimersProps) {
  const { timers, receivedAtMs } = shown;
  const isCounting = timers.some((timer) => timer.state === 'running');
  const now = Math.max(useClock(isCounting), receivedAtMs);
  // The running row whose cancel question is open. The question closes for
  // good when its timer ends or leaves the list (React's "adjust state while
  // rendering": no effect, no extra frame with a stale question).
  const [askingId, setAskingId] = useState<number | null>(null);
  if (askingId !== null && !timers.some((timer) => timer.id === askingId && timer.state === 'running')) {
    setAskingId(null);
  }
  if (timers.length === 0) {
    return null;
  }
  const closeQuestion = () => setAskingId(null);
  return (
    <section
      className="timers"
      aria-label="Timers"
      style={{ ...PANEL_COLOR_STYLE, ...TIMER_STRIP_BOX }}
    >
      <ol className="timers-rows">
        {timers.map((timer) => {
          const name = timerName(timer.label);
          switch (timer.state) {
            case 'pending':
              return <PendingRow key={timer.id} timer={timer} name={name} onConfirm={onConfirm} onReject={onReject} />;
            case 'running':
              return askingId === timer.id ? (
                <CancelQuestion key={timer.id} timer={timer} name={name} onCancel={onCancel} onClose={closeQuestion} />
              ) : (
                <TapRow
                  key={timer.id}
                  timer={timer}
                  name={name}
                  value={formatCountdown(timer.endsAtMs - now)}
                  onTap={() => setAskingId(timer.id)}
                />
              );
            case 'ended':
              return (
                <TapRow key={timer.id} timer={timer} name={name} value="Ended" onTap={() => onCancel(timer.id)} />
              );
          }
        })}
      </ol>
    </section>
  );
}

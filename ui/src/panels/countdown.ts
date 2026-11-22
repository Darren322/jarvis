/**
 * How the timer strip writes a timer: its name and its time left. Pure
 * functions only, so each rule is tested once in a table.
 */

/** The name of a timer Jarvis sent with no label, or with an empty one. */
export const UNLABELLED_TIMER = 'Timer';

/** How often the strip reads the clock while a row counts down, in milliseconds. */
export const COUNTDOWN_TICK_MS = 1000;

const MS_PER_SECOND = 1000;
const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 3600;

/** The name the strip shows for a timer: its label, or "Timer" when it has none. Shown as text, never as markup. */
export function timerName(label: string | null): string {
  return label === null || label.trim() === '' ? UNLABELLED_TIMER : label;
}

function twoDigits(value: number): string {
  return String(value).padStart(2, '0');
}

/**
 * A timer's time left as the strip shows it: "m:ss" under an hour, "h:mm:ss"
 * from an hour on. Whole seconds round up, so a row reads "0:01" through its
 * last second and "0:00" from the end on; never negative.
 */
export function formatCountdown(remainingMs: number): string {
  const total = Math.max(0, Math.ceil(remainingMs / MS_PER_SECOND));
  const hours = Math.floor(total / SECONDS_PER_HOUR);
  const minutes = Math.floor((total % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE);
  const seconds = twoDigits(total % SECONDS_PER_MINUTE);
  return hours > 0 ? `${hours}:${twoDigits(minutes)}:${seconds}` : `${minutes}:${seconds}`;
}

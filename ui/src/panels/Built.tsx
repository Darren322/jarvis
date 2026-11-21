/**
 * A built Panel (jarvis-ui 12): the title and 1 to 4 parts the model chose
 * from the Catalog, laid out in the same frame as the Dashboard. It lays out
 * what it is sent and nothing else (D2): card and list text is the model's,
 * rendered as React text (never as markup), and a gauge reads its value from
 * the latest Stats exactly as the Dashboard does (`Gauge`).
 */
import type { BuiltPanel, PanelPart, Stats } from '../link/protocol';
import { Gauge, Heading } from './Dashboard';

export interface BuiltProps {
  readonly panel: BuiltPanel;
  /** The latest Stats reading, or `null` before the first one: every gauge then shows `WAITING`. */
  readonly reading: Stats | null;
}

/** A card or list title: the Dashboard's label style. */
function PartTitle({ title }: { readonly title: string }) {
  return <h3 className="panel-label built-part-title">{title}</h3>;
}

function Part({ part, reading }: { readonly part: PanelPart; readonly reading: Stats | null }) {
  switch (part.kind) {
    case 'card':
      return (
        <article className="built-part built-card">
          <PartTitle title={part.title} />
          <p className="built-text">{part.text}</p>
        </article>
      );
    case 'list':
      return (
        <article className="built-part built-list">
          <PartTitle title={part.title} />
          <ul className="built-items">
            {part.items.map((item, index) => (
              // Items hold no state, so their place is their identity.
              <li key={index} className="built-item">
                {item}
              </li>
            ))}
          </ul>
        </article>
      );
    case 'gauge':
      return (
        <dl className="built-part built-gauge" aria-busy={reading === null}>
          <Gauge source={part.source} reading={reading} />
        </dl>
      );
  }
}

/**
 * The parts in a fixed grid that never grows past the frame: one part fills
 * it, two sit side by side, three or four make two rows of two (a third part
 * spans the row). Text that does not fit is cut with an ellipsis.
 */
export function Built({ panel, reading }: BuiltProps) {
  return (
    <>
      <Heading title={panel.title} />
      <div className={`built-parts built-parts-${panel.parts.length}`}>
        {panel.parts.map((part, index) => (
          // Parts hold no state, so their place is their identity.
          <Part key={index} part={part} reading={reading} />
        ))}
      </div>
    </>
  );
}

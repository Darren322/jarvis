/** The text Darren reads next to `label` (a `dt`): its first `dd`. Fails the test if there is none. */
export function valueOf(root: ParentNode, label: string): string {
  const term = [...root.querySelectorAll('dt')].find((element) => element.textContent === label);
  if (term === undefined) {
    throw new Error(`no value is labelled ${label}`);
  }
  const value = term.nextElementSibling;
  if (value === null || value.tagName !== 'DD') {
    throw new Error(`${label} has no value`);
  }
  return value.textContent ?? '';
}

/** Every value on the Dashboard by its label, in screen order. */
export function allValues(root: ParentNode): Record<string, string> {
  return Object.fromEntries(
    [...root.querySelectorAll('dt')].map((term) => [term.textContent ?? '', valueOf(root, term.textContent ?? '')]),
  );
}

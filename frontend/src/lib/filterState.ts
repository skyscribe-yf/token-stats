export interface AppliedRange {
  from: string;
  to: string;
  appliedAt: number;
}

export function buildCsvFilterParam(
  selected: ReadonlySet<string>,
  options: readonly string[]
): string | undefined {
  if (options.length === 0) return undefined;

  const selectedInOptionOrder = options.filter((option) => selected.has(option));
  if (selectedInOptionOrder.length === 0) return undefined;
  if (selectedInOptionOrder.length === options.length) return undefined;

  return selectedInOptionOrder.join(",");
}

export function isEmptyAppliedSelection(
  selected: ReadonlySet<string>,
  options: readonly string[]
): boolean {
  if (options.length === 0) return false;
  return options.every((option) => !selected.has(option));
}

export function formatAppliedRange(
  presetLabel: string,
  range: Pick<AppliedRange, "from" | "to">
): string {
  return `${presetLabel} · ${formatBound(range.from)} 至 ${formatBound(range.to)}`;
}

function formatBound(value: string): string {
  return value.replace("T", " ");
}

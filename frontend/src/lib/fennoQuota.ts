export function remainingQuota(limit: number | null, used: number): number | null {
  if (limit == null) return null;
  return Math.max(limit - used, 0);
}

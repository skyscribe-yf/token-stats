import { useState, useCallback } from "react";
import {
  fetchStats,
  fetchFilters,
  type StatsResponse,
  type FilterOptions,
} from "../api";
import { emptyStatsResponse } from "./shared";

interface UseStatsParams {
  from: string;
  to: string;
  sourceFilter: string | undefined;
  vendorFilter: string | undefined;
  tzOffset: number;
  hasEmptyRequiredSelection: boolean;
}

export function useStats() {
  const [stats, setStats] = useState<StatsResponse | null>(null);
  const [filters, setFilters] = useState<FilterOptions>({
    vendors: [],
    models: [],
    sources: [],
  });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [lastUpdatedAt, setLastUpdatedAt] = useState<Date | null>(null);

  const loadData = useCallback(
    async ({
      from,
      to,
      sourceFilter,
      vendorFilter,
      tzOffset,
      hasEmptyRequiredSelection,
    }: UseStatsParams) => {
      if (!from || !to) return;

      if (hasEmptyRequiredSelection) {
        setStats(emptyStatsResponse());
        setLastUpdatedAt(new Date());
        return;
      }

      setLoading(true);
      setError(null);
      try {
        const [s, f] = await Promise.all([
          fetchStats(from, to, sourceFilter, vendorFilter, tzOffset),
          fetchFilters(),
        ]);
        setStats(s);
        setFilters(f);
        setLastUpdatedAt(new Date());
      } catch (e) {
        setError(e instanceof Error ? e.message : "加载数据失败");
      } finally {
        setLoading(false);
      }
    },
    []
  );

  return { stats, filters, setFilters, loading, error, lastUpdatedAt, loadData };
}

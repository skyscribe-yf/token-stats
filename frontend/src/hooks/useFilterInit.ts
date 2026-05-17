import { useState, useEffect, useRef } from "react";
import type { FilterOptions } from "../api";

/**
 * One-time filter initialization: when filters are first loaded,
 * select all sources and vendors by default.
 *
 * Replaces the `filtersInitializedRef` anti-pattern.
 */
export function useFilterInit(filters: FilterOptions | null) {
  const didInit = useRef(false);

  const [selectedSources, setSelectedSources] = useState<Set<string>>(
    new Set()
  );
  const [selectedVendors, setSelectedVendors] = useState<Set<string>>(
    new Set()
  );

  useEffect(() => {
    if (!filters || didInit.current) return;
    didInit.current = true;
    setSelectedSources(new Set(filters.sources));
    setSelectedVendors(new Set(filters.vendors));
  }, [filters]);

  return {
    selectedSources,
    setSelectedSources,
    selectedVendors,
    setSelectedVendors,
    didInit: didInit.current,
  };
}

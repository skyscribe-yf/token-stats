import { useState, useCallback } from "react";
import {
  fetchRequests,
  type PaginatedRequests,
} from "../api";
import { emptyRequests } from "./shared";

interface UseRequestsParams {
  from: string;
  to: string;
  vendorFilter: string | undefined;
  selectedModel: string | undefined;
  sourceFilter: string | undefined;
  page: number;
  tzOffset: number;
  hasEmptyRequiredSelection: boolean;
}

export function useRequests() {
  const [requests, setRequests] = useState<PaginatedRequests | null>(null);

  const loadRequests = useCallback(
    async ({
      from,
      to,
      vendorFilter,
      selectedModel,
      sourceFilter,
      page,
      tzOffset,
      hasEmptyRequiredSelection,
    }: UseRequestsParams) => {
      if (!from || !to) return;

      if (hasEmptyRequiredSelection) {
        setRequests(emptyRequests(1));
        return;
      }

      try {
        const r = await fetchRequests(
          from,
          to,
          vendorFilter,
          selectedModel || undefined,
          sourceFilter,
          page,
          50,
          tzOffset
        );
        setRequests(r);
      } catch (e) {
        console.error("Failed to load requests", e);
      }
    },
    []
  );

  return { requests, loadRequests };
}

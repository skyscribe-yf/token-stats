import { useState, useEffect } from "react";
import { fetchQuota, type QuotaResponse } from "../api";

export function useQuota() {
  const [quota, setQuota] = useState<QuotaResponse | null>(null);
  const [quotaLoading, setQuotaLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    const loadQuota = async () => {
      try {
        const q = await fetchQuota();
        if (!cancelled) setQuota(q);
      } catch {
        /* quota is optional — don't set error state */
      } finally {
        if (!cancelled) setQuotaLoading(false);
      }
    };
    loadQuota();
    const interval = setInterval(loadQuota, 60_000);
    return () => {
      cancelled = true;
      clearInterval(interval);
    };
  }, []);

  return { quota, quotaLoading };
}

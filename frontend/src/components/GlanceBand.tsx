import { memo } from "react";
import { KpiStrip } from "./KpiStrip";
import { QuotaChips } from "./QuotaChips";
import type { AggregatedStats } from "../api";
import type {
  QuotaResponse,
  XunfeiMultiStatus,
  AinaibaCreditResponse,
} from "../api";

interface GlanceBandProps {
  overall: AggregatedStats;
  quota: QuotaResponse | null;
  xunfei: XunfeiMultiStatus | null;
  ainaibaCredit: AinaibaCreditResponse | null;
  quotaLoading: boolean;
  hiddenCards: Set<string>;
  onChipClick: (cardId: string) => void;
}

export const GlanceBand = memo(function GlanceBand({
  overall,
  quota,
  xunfei,
  ainaibaCredit,
  quotaLoading,
  hiddenCards,
  onChipClick,
}: GlanceBandProps) {
  return (
    <section aria-label="Glance" className="space-y-3">
      <QuotaChips
        quota={quota}
        xunfei={xunfei}
        ainaibaCredit={ainaibaCredit}
        loading={quotaLoading}
        hiddenCards={hiddenCards}
        onChipClick={onChipClick}
      />
      <KpiStrip overall={overall} />
    </section>
  );
});

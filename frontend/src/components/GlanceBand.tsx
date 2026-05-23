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
  onChipClick: (cardId: string) => void;
}

export function GlanceBand({
  overall,
  quota,
  xunfei,
  ainaibaCredit,
  quotaLoading,
  onChipClick,
}: GlanceBandProps) {
  return (
    <section aria-label="Glance" className="space-y-3">
      <QuotaChips
        quota={quota}
        xunfei={xunfei}
        ainaibaCredit={ainaibaCredit}
        loading={quotaLoading}
        onChipClick={onChipClick}
      />
      <KpiStrip overall={overall} />
    </section>
  );
}

import { getVendorColor } from "../lib/utils";

const SUBSCRIPTION_VENDORS = ["kimi", "xunfei", "opencode-go", "opencode"];

interface SidebarVendorListProps {
  vendors: string[];
  selectedVendors: ReadonlySet<string>;
  onToggle: (vendor: string) => void;
  onToggleSubscriptionGroup: (selectAll: boolean) => void;
}

export function SidebarVendorList({
  vendors,
  selectedVendors,
  onToggle,
  onToggleSubscriptionGroup,
}: SidebarVendorListProps) {
  if (vendors.length === 0) return null;

  const regularVendors = vendors.filter((v) => !SUBSCRIPTION_VENDORS.includes(v));
  const subVendors = vendors.filter((v) => SUBSCRIPTION_VENDORS.includes(v));
  const allSubSelected =
    subVendors.length > 0 && subVendors.every((v) => selectedVendors.has(v));

  return (
    <div className="py-3 px-3">
      <p className="text-[10px] uppercase tracking-wider text-slate-400 font-semibold mb-1.5 px-2">
        供应商
      </p>
      <div className="space-y-0.5">
        {regularVendors.map((v) => (
          <VendorRow
            key={v}
            vendor={v}
            selected={selectedVendors.has(v)}
            onToggle={onToggle}
          />
        ))}
      </div>

      {subVendors.length > 0 && (
        <>
          <div className="mt-3 px-2 mb-1 flex items-center justify-between">
            <p className="text-[10px] uppercase tracking-wider text-slate-400 font-semibold">
              订阅
            </p>
            <button
              onClick={() => onToggleSubscriptionGroup(!allSubSelected)}
              className="text-[10px] text-primary-600 hover:text-primary-700 font-medium"
            >
              {allSubSelected ? "取消全选" : "全选"}
            </button>
          </div>
          <div className="space-y-0.5">
            {subVendors.map((v) => (
              <VendorRow
                key={v}
                vendor={v}
                selected={selectedVendors.has(v)}
                onToggle={onToggle}
              />
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function VendorRow({
  vendor,
  selected,
  onToggle,
}: {
  vendor: string;
  selected: boolean;
  onToggle: (v: string) => void;
}) {
  return (
    <label className="flex items-center gap-2 px-2 py-1.5 text-xs text-slate-700 hover:bg-slate-50 rounded cursor-pointer">
      <input
        type="checkbox"
        checked={selected}
        onChange={() => onToggle(vendor)}
        className="rounded border-slate-300 text-primary-600 focus:ring-primary-500"
      />
      <span
        className="w-2 h-2 rounded-full shrink-0"
        style={{ background: getVendorColor(vendor) }}
      />
      <span className="truncate">{vendor}</span>
    </label>
  );
}

export function QuotaCard({
  title,
  provider,
  status,
  children,
}: {
  title: string;
  provider: string;
  status: "available" | "unavailable" | "loading";
  children: React.ReactNode;
}) {
  const borderColor =
    status === "available" ? "border-emerald-200" : "border-slate-200";
  const indicatorColor =
    status === "available"
      ? "bg-emerald-500"
      : status === "loading"
        ? "bg-amber-400"
        : "bg-slate-300";
  return (
    <div className={`bg-white rounded-xl border ${borderColor} p-4 shadow-sm`}>
      <div className="flex items-center justify-between mb-3">
        <div className="flex items-center gap-2">
          <span className={`w-2 h-2 rounded-full ${indicatorColor}`} />
          <span className="text-xs font-semibold text-slate-700">{title}</span>
        </div>
        <span className="text-[10px] text-slate-400">{provider}</span>
      </div>
      {children}
    </div>
  );
}

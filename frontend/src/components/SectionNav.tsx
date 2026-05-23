export type SectionId = "usage" | "quotas" | "requests";

interface SectionNavProps {
  active: SectionId;
  onSelect: (id: SectionId) => void;
}

const SECTIONS: { id: SectionId; label: string }[] = [
  { id: "usage", label: "用量" },
  { id: "quotas", label: "订阅" },
  { id: "requests", label: "请求" },
];

export function SectionNav({ active, onSelect }: SectionNavProps) {
  return (
    <nav
      aria-label="Section navigation"
      className="sticky top-11 z-20 -mx-4 px-4 py-2 bg-slate-50/95 backdrop-blur border-b border-slate-200"
    >
      <div className="inline-flex items-center bg-slate-100 rounded-lg p-0.5 gap-0.5">
        {SECTIONS.map((s) => (
          <button
            key={s.id}
            onClick={() => onSelect(s.id)}
            className={`px-3 py-1 text-xs font-medium rounded-md transition-colors ${
              active === s.id
                ? "bg-white text-primary-700 shadow-sm"
                : "text-slate-500 hover:text-slate-700"
            }`}
          >
            {s.label}
          </button>
        ))}
      </div>
    </nav>
  );
}

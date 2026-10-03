import type { ReactNode } from "react";

export function Card({ title, children, className = "" }: { title: string; children: ReactNode; className?: string }) {
  return (
    <section className={`rounded-xl bg-white p-4 shadow-sm dark:bg-panel-dark ${className}`}>
      <h3 className="mb-3 text-xs font-semibold uppercase tracking-wide text-slate-400">{title}</h3>
      {children}
    </section>
  );
}

export function Meter({
  label,
  value,
  detail,
  ratio,
  color = "bg-accent",
}: {
  label: string;
  value: string;
  detail?: string;
  ratio: number;
  color?: string;
}) {
  const width = `${Math.max(0, Math.min(1, ratio)) * 100}%`;
  return (
    <div className="mb-3 last:mb-0">
      <div className="mb-1 flex items-baseline justify-between gap-2">
        <span className="text-sm text-slate-500 dark:text-slate-400">{label}</span>
        <span className="font-semibold tabular-nums">{value}</span>
      </div>
      <div className="h-2 overflow-hidden rounded-full bg-slate-100 dark:bg-panel-2-dark">
        <div className={`h-full rounded-full ${color} transition-all duration-500`} style={{ width }} />
      </div>
      {detail && <div className="mt-1 text-xs text-slate-400">{detail}</div>}
    </div>
  );
}

export function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div>
      <div className="text-xs text-slate-400">{label}</div>
      <div className="text-xl font-semibold tabular-nums">{value}</div>
      {hint && <div className="text-xs text-slate-400">{hint}</div>}
    </div>
  );
}

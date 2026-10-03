import type { ProfileTurn } from "../lib/api";

const SERIES: { key: keyof ProfileTurn | "other"; label: string; color: string }[] = [
  { key: "expert_wait_s", label: "NVMe wait", color: "#f59e0b" },
  { key: "expert_matmul_s", label: "Expert matmul", color: "#14b8a6" },
  { key: "attention_s", label: "Attention", color: "#6366f1" },
  { key: "lm_head_s", label: "LM head", color: "#ec4899" },
  { key: "other", label: "Other", color: "#94a3b8" },
];

function parts(turn: ProfileTurn): number[] {
  const known = turn.expert_wait_s + turn.expert_matmul_s + turn.attention_s + turn.lm_head_s;
  return [turn.expert_wait_s, turn.expert_matmul_s, turn.attention_s, turn.lm_head_s, Math.max(0, turn.wall_s - known)];
}

export default function TurnChart({ turns }: { turns: ProfileTurn[] }) {
  if (turns.length === 0) {
    return <div className="text-sm text-slate-400">No turns profiled yet.</div>;
  }
  const width = 600;
  const height = 160;
  const max = Math.max(...turns.map((t) => t.wall_s), 0.001);
  const slot = width / Math.max(turns.length, 10);
  const bar = Math.max(2, slot * 0.7);

  return (
    <div>
      <svg viewBox={`0 0 ${width} ${height}`} className="h-40 w-full" preserveAspectRatio="none">
        {turns.map((turn, i) => {
          let y = height;
          return (
            <g key={i}>
              <title>{`${turn.wall_s.toFixed(1)}s, ${turn.completion_tokens} tokens`}</title>
              {parts(turn).map((value, j) => {
                const h = (value / max) * (height - 4);
                y -= h;
                return <rect key={j} x={i * slot} y={y} width={bar} height={h} fill={SERIES[j].color} />;
              })}
            </g>
          );
        })}
      </svg>
      <div className="mt-2 flex flex-wrap gap-4 text-xs text-slate-400">
        {SERIES.map((s) => (
          <span key={s.label} className="flex items-center gap-1.5">
            <span className="h-2.5 w-2.5 rounded-sm" style={{ background: s.color }} />
            {s.label}
          </span>
        ))}
        <span className="ml-auto">longest turn {max.toFixed(1)}s</span>
      </div>
    </div>
  );
}

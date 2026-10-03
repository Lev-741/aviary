import { useEffect, useRef, useState } from "react";
import type { ExpertSnapshot } from "../lib/api";

const TIER_COLORS: [number, number, number][] = [
  [148, 163, 184],
  [20, 184, 166],
  [99, 102, 241],
];
const TIER_NAMES = ["NVMe", "RAM", "VRAM"];

function bytes(hex: string): Uint8Array {
  const out = new Uint8Array(Math.floor(hex.length / 2));
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.substr(i * 2, 2), 16) || 0;
  return out;
}

export default function ExpertGrid({ snapshot }: { snapshot: ExpertSnapshot }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [tip, setTip] = useState<string | null>(null);
  const { rows, cols } = snapshot;
  const cell = cols > 128 ? 3 : cols > 64 ? 4 : 6;

  useEffect(() => {
    const el = canvas.current;
    if (!el || rows === 0 || cols === 0) return;
    const ctx = el.getContext("2d");
    if (!ctx) return;
    const map = bytes(snapshot.map);
    const hits = bytes(snapshot.hits);
    const image = ctx.createImageData(cols * cell, rows * cell);
    for (let r = 0; r < rows; r++) {
      for (let c = 0; c < cols; c++) {
        const i = r * cols + c;
        const b = map[i] ?? 0;
        const tier = Math.min(2, b >> 6);
        const heat = b & 63;
        const hit = (hits[i >> 3] ?? 0) & (1 << (i & 7));
        const shade = 0.35 + 0.65 * Math.min(1, heat / 24);
        const [cr, cg, cb] = hit ? [255, 255, 255] : TIER_COLORS[tier].map((v) => v * shade);
        for (let y = 0; y < cell; y++) {
          for (let x = 0; x < cell; x++) {
            if (x === cell - 1 || y === cell - 1) continue;
            const p = ((r * cell + y) * cols * cell + c * cell + x) * 4;
            image.data[p] = cr;
            image.data[p + 1] = cg;
            image.data[p + 2] = cb;
            image.data[p + 3] = 255;
          }
        }
      }
    }
    ctx.putImageData(image, 0, 0);
  }, [snapshot, rows, cols, cell]);

  if (rows === 0 || cols === 0 || !snapshot.map) {
    return <div className="text-sm text-slate-400">No expert map yet. Colibri publishes it after the first turn, and only to authenticated clients.</div>;
  }

  const onMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const c = Math.floor(((e.clientX - rect.left) / rect.width) * cols);
    const r = Math.floor(((e.clientY - rect.top) / rect.height) * rows);
    const i = r * cols + c;
    const b = parseInt(snapshot.map.substr(i * 2, 2), 16) || 0;
    setTip(`layer ${r}, expert ${c}: ${TIER_NAMES[Math.min(2, b >> 6)]}, heat ${b & 63}`);
  };

  return (
    <div>
      <div className="scroll-thin overflow-auto">
        <canvas
          ref={canvas}
          width={cols * cell}
          height={rows * cell}
          onMouseMove={onMove}
          onMouseLeave={() => setTip(null)}
          className="block"
          style={{ imageRendering: "pixelated" }}
        />
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-4 text-xs text-slate-400">
        {TIER_NAMES.map((name, i) => (
          <span key={name} className="flex items-center gap-1.5">
            <span className="h-2.5 w-2.5 rounded-sm" style={{ background: `rgb(${TIER_COLORS[i].join(",")})` }} />
            {name}
          </span>
        ))}
        <span className="flex items-center gap-1.5">
          <span className="h-2.5 w-2.5 rounded-sm border border-slate-300 bg-white" />
          routed last turn
        </span>
        <span className="ml-auto tabular-nums">{tip ?? `${rows} layers x ${cols} experts`}</span>
      </div>
    </div>
  );
}

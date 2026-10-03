import { Card, Meter, Stat } from "../components/Meter";
import ExpertGrid from "../components/ExpertGrid";
import TurnChart from "../components/TurnChart";
import { RefreshIcon } from "../components/Icons";
import { api } from "../lib/api";
import { usePolling } from "../lib/usePolling";
import { count, gb, percent, seconds } from "../lib/format";

const STATE_STYLE: Record<string, string> = {
  idle: "bg-emerald-100 text-emerald-700 dark:bg-emerald-900/40 dark:text-emerald-300",
  generating: "bg-sky-100 text-sky-700 dark:bg-sky-900/40 dark:text-sky-300",
  queued: "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-300",
  unknown: "bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-300",
};

export default function MonitorPage() {
  const status = usePolling(api.getStatus, 3000);
  const experts = usePolling(api.getExperts, 3000);
  const payload = status.data;

  if (!payload) {
    return (
      <div className="flex flex-1 items-center justify-center bg-slate-50 text-slate-400 dark:bg-chat-bg-dark">
        {status.error ?? "Loading Colibri status..."}
      </div>
    );
  }

  const s = payload.status;
  const health = s.health;
  const hw = health?.hwinfo;
  const tiers = health?.tiers;
  const scheduler = health?.scheduler;
  const turn = s.last_turn;
  const routing = s.routing;
  const cache = payload.cache;
  const ramUsed = hw ? Math.max(0, hw.ram_total_gb - hw.ram_avail_gb) : 0;
  const hitRate = routing && routing.routed_last_turn > 0 ? routing.served_from_memory / routing.routed_last_turn : null;
  const warmTotal = cache.warm_turns + cache.cold_turns;

  return (
    <div className="scroll-thin flex-1 overflow-y-auto bg-slate-50 p-6 dark:bg-chat-bg-dark">
      <div className="mx-auto flex max-w-6xl flex-col gap-4">
        <header className="flex flex-wrap items-center gap-3">
          <h1 className="text-xl font-semibold">Colibri</h1>
          <span className={`rounded-full px-2.5 py-0.5 text-xs font-medium ${s.reachable ? STATE_STYLE[s.state] : "bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300"}`}>
            {s.reachable ? s.state : "offline"}
          </span>
          <span className="text-sm text-slate-400">{s.base_url}</span>
          <span className="text-sm text-slate-400">
            {s.configured_model}
            {s.reachable && !s.models.some((m) => m.id === s.configured_model) && s.models.length > 0 && " (not served)"}
          </span>
          <button
            onClick={() => {
              status.refresh();
              experts.refresh();
            }}
            className="ml-auto rounded-full p-2 text-slate-400 hover:bg-white hover:text-accent dark:hover:bg-panel-dark"
            title="Refresh"
          >
            <RefreshIcon width={18} height={18} />
          </button>
        </header>

        {!s.reachable && (
          <div className="rounded-xl bg-red-50 p-4 text-sm text-red-700 dark:bg-red-900/30 dark:text-red-300">
            {s.error ?? "Colibri is not reachable."} Start it with <code className="font-mono">coli serve</code> or check the URL in Settings.
          </div>
        )}

        {s.reachable && !health?.hwinfo && !health?.tiers && (
          <div className="rounded-xl bg-amber-50 p-4 text-sm text-amber-800 dark:bg-amber-900/30 dark:text-amber-200">
            Colibri only shares hardware and expert details with authenticated clients. Set the API key in Settings if the server uses COLI_API_KEY.
          </div>
        )}

        <div className="grid grid-cols-2 gap-4 lg:grid-cols-4">
          <Card title="Decode speed">
            <Stat
              label="last turn"
              value={turn?.completion_tokens && turn.wall_s ? `${(turn.completion_tokens / turn.wall_s).toFixed(2)} tok/s` : "-"}
              hint={turn ? `${turn.prompt_tokens} prompt + ${turn.completion_tokens} generated in ${seconds(turn.wall_s)}` : undefined}
            />
          </Card>
          <Card title="Expert hit rate">
            <Stat
              label="routed experts already in memory"
              value={hitRate === null ? "-" : percent(hitRate)}
              hint={routing ? `${routing.routed_last_turn} routed, ${routing.streamed_from_disk} from NVMe` : undefined}
            />
          </Card>
          <Card title="NVMe streaming">
            <Stat
              label="time blocked on disk"
              value={turn ? percent(s.nvme.last_turn_io_wait_share) : "-"}
              hint={turn ? `disk reads ${seconds(s.nvme.last_turn_disk_s)} last turn` : undefined}
            />
          </Card>
          <Card title="KV slot reuse">
            <Stat
              label="turns that hit a warm slot"
              value={warmTotal ? percent(cache.warm_turns / warmTotal) : "-"}
              hint={`${health?.kv_slots ?? cache.slots.length} slots on the server`}
            />
          </Card>
        </div>

        <div className="grid gap-4 lg:grid-cols-3">
          <Card title="Memory">
            {hw ? (
              <>
                <Meter label="RAM" value={`${gb(ramUsed)} / ${gb(hw.ram_total_gb)}`} ratio={hw.ram_total_gb ? ramUsed / hw.ram_total_gb : 0} detail={`${hw.cpu}, ${hw.cores} cores`} />
                <Meter
                  label="VRAM experts"
                  value={hw.vram_total_gb ? `${gb(tiers?.vram_gb ?? 0)} / ${gb(hw.vram_total_gb)}` : "no GPU"}
                  ratio={hw.vram_total_gb ? (tiers?.vram_gb ?? 0) / hw.vram_total_gb : 0}
                  color="bg-tier-vram"
                  detail={hw.gpus ? `${hw.gpus} GPU: ${hw.gpu}` : undefined}
                />
                <Meter label="RAM experts" value={gb(tiers?.ram_gb ?? 0)} ratio={hw.ram_total_gb ? (tiers?.ram_gb ?? 0) / hw.ram_total_gb : 0} color="bg-tier-ram" />
              </>
            ) : (
              <div className="text-sm text-slate-400">No hardware info.</div>
            )}
          </Card>
          <Card title="Expert placement">
            {tiers ? (
              <>
                <div className="mb-3 flex h-3 overflow-hidden rounded-full">
                  {(["vram", "ram", "disk"] as const).map((k) => (
                    <div
                      key={k}
                      className={k === "vram" ? "bg-tier-vram" : k === "ram" ? "bg-tier-ram" : "bg-tier-disk"}
                      style={{ width: `${(tiers[k] / Math.max(1, tiers.vram + tiers.ram + tiers.disk)) * 100}%` }}
                    />
                  ))}
                </div>
                <div className="grid grid-cols-3 gap-2 text-sm">
                  <Stat label="VRAM" value={count(tiers.vram)} />
                  <Stat label="RAM" value={count(tiers.ram)} />
                  <Stat label="NVMe" value={count(tiers.disk)} />
                </div>
                <div className="mt-2 text-xs text-slate-400">{percent(s.nvme.disk_ratio)} of experts are streamed from NVMe on demand.</div>
              </>
            ) : (
              <div className="text-sm text-slate-400">No tier info.</div>
            )}
          </Card>
          <Card title="Scheduler">
            {scheduler ? (
              <div className="grid grid-cols-2 gap-3 text-sm">
                <Stat label="active" value={`${scheduler.active} / ${scheduler.capacity}`} />
                <Stat label="queued" value={`${scheduler.queued} / ${scheduler.max_queue}`} />
                <Stat label="completed" value={count(scheduler.completed)} />
                <Stat label="failed" value={count(scheduler.failed)} />
                <Stat label="rejected" value={count(scheduler.rejected)} />
                <Stat label="timed out" value={count(scheduler.timed_out)} />
              </div>
            ) : (
              <div className="text-sm text-slate-400">No scheduler info.</div>
            )}
          </Card>
        </div>

        <Card title="Where each turn spends its time">
          <TurnChart turns={s.recent_turns} />
        </Card>

        <Card title="Expert map">
          {experts.data ? <ExpertGrid snapshot={experts.data} /> : <div className="text-sm text-slate-400">{experts.error ?? "Loading..."}</div>}
        </Card>

        <Card title="KV slots">
          <table className="w-full text-sm">
            <thead className="text-left text-xs text-slate-400">
              <tr>
                <th className="pb-2 font-normal">slot</th>
                <th className="pb-2 font-normal">conversation</th>
                <th className="pb-2 font-normal">turns</th>
                <th className="pb-2 font-normal">idle</th>
                <th className="pb-2 font-normal">expert hit rate</th>
              </tr>
            </thead>
            <tbody className="tabular-nums">
              {cache.slots.map((slot) => (
                <tr key={slot.slot} className="border-t border-slate-100 dark:border-panel-2-dark">
                  <td className="py-1.5">{slot.slot}</td>
                  <td className="truncate py-1.5">{slot.owner ?? <span className="text-slate-400">free</span>}</td>
                  <td className="py-1.5">{slot.turns}</td>
                  <td className="py-1.5">{slot.idle_secs === null ? "-" : `${slot.idle_secs}s`}</td>
                  <td className="py-1.5">{slot.last_hit_rate === null ? "-" : percent(slot.last_hit_rate)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      </div>
    </div>
  );
}

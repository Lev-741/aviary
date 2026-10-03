import { api } from "../lib/api";
import { usePolling } from "../lib/usePolling";
import { useStore } from "../lib/store";

export default function StatusDot() {
  const { data } = usePolling(api.getStatus, 10_000);
  const setPage = useStore((s) => s.setPage);
  const status = data?.status;
  const online = status?.reachable ?? false;
  const busy = status?.state === "generating" || status?.state === "queued";
  const color = !status ? "bg-slate-500" : !online ? "bg-red-500" : busy ? "bg-amber-400" : "bg-emerald-400";
  const label = !status ? "Checking Colibri" : !online ? "Colibri offline" : busy ? `Colibri ${status.state}` : "Colibri idle";

  return (
    <button onClick={() => setPage("monitor")} title={label} className="flex flex-col items-center gap-1 text-[10px]">
      <span className={`h-3 w-3 rounded-full ${color} ${busy ? "animate-pulse" : ""}`} />
      {online ? "online" : "offline"}
    </button>
  );
}

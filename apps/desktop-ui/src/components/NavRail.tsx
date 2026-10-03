import type { ComponentType, SVGProps } from "react";
import { ChatIcon, GearIcon, PulseIcon } from "./Icons";
import StatusDot from "./StatusDot";
import { useStore, type Page } from "../lib/store";

const items: { page: Page; label: string; icon: ComponentType<SVGProps<SVGSVGElement>> }[] = [
  { page: "chats", label: "Chats", icon: ChatIcon },
  { page: "monitor", label: "Monitor", icon: PulseIcon },
  { page: "settings", label: "Settings", icon: GearIcon },
];

export default function NavRail() {
  const page = useStore((s) => s.page);
  const setPage = useStore((s) => s.setPage);

  return (
    <nav className="flex w-[72px] shrink-0 flex-col items-center gap-1 bg-[#2a3a4c] py-3 text-slate-300 dark:bg-[#0e1621]">
      <img src="/icon.png" alt="Aviary" className="mb-3 h-10 w-10 rounded-xl" draggable={false} />
      {items.map(({ page: target, label, icon: Icon }) => (
        <button
          key={target}
          onClick={() => setPage(target)}
          className={`flex w-16 flex-col items-center gap-1 rounded-lg py-2 text-[11px] transition ${
            page === target ? "bg-white/10 text-white" : "hover:bg-white/5 hover:text-white"
          }`}
        >
          <Icon />
          {label}
        </button>
      ))}
      <div className="mt-auto">
        <StatusDot />
      </div>
    </nav>
  );
}

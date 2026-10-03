import { useMemo, useState } from "react";
import { PlusIcon, SearchIcon } from "./Icons";
import { useStore } from "../lib/store";
import { avatarColor, initials, shortDate } from "../lib/format";

export default function ConversationList() {
  const conversations = useStore((s) => s.conversations);
  const drafts = useStore((s) => s.drafts);
  const activeId = useStore((s) => s.activeId);
  const busy = useStore((s) => s.busy);
  const open = useStore((s) => s.openConversation);
  const create = useStore((s) => s.newConversation);
  const [query, setQuery] = useState("");

  const items = useMemo(() => {
    const all = [...drafts, ...conversations];
    const q = query.trim().toLowerCase();
    if (!q) return all;
    return all.filter((c) => c.title.toLowerCase().includes(q) || c.last_message.toLowerCase().includes(q));
  }, [conversations, drafts, query]);

  return (
    <aside className="flex w-[300px] shrink-0 flex-col border-r border-slate-200 bg-white dark:border-black/40 dark:bg-panel-dark">
      <div className="flex items-center gap-2 p-3">
        <div className="flex flex-1 items-center gap-2 rounded-full bg-slate-100 px-3 py-1.5 text-slate-500 dark:bg-panel-2-dark dark:text-slate-400">
          <SearchIcon width={16} height={16} />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search"
            className="w-full bg-transparent text-sm text-slate-800 outline-none placeholder:text-slate-400 dark:text-slate-100"
          />
        </div>
        <button
          onClick={create}
          title="New chat"
          className="flex h-8 w-8 items-center justify-center rounded-full bg-accent text-white hover:bg-accent-dark"
        >
          <PlusIcon width={18} height={18} />
        </button>
      </div>
      <ul className="scroll-thin flex-1 overflow-y-auto">
        {items.length === 0 && (
          <li className="px-4 py-8 text-center text-sm text-slate-400">No chats yet</li>
        )}
        {items.map((c) => {
          const title = c.title || "New chat";
          const active = c.id === activeId;
          return (
            <li key={c.id}>
              <button
                onClick={() => open(c.id)}
                className={`flex w-full items-center gap-3 px-3 py-2 text-left ${
                  active ? "bg-accent text-white" : "hover:bg-slate-100 dark:hover:bg-panel-2-dark"
                }`}
              >
                <span
                  className="flex h-12 w-12 shrink-0 items-center justify-center rounded-full text-base font-semibold text-white"
                  style={{ background: avatarColor(c.id) }}
                >
                  {initials(title)}
                </span>
                <span className="min-w-0 flex-1">
                  <span className="flex items-baseline justify-between gap-2">
                    <span className="truncate font-medium">{title}</span>
                    <span className={`shrink-0 text-xs ${active ? "text-white/80" : "text-slate-400"}`}>
                      {shortDate(c.updated_at)}
                    </span>
                  </span>
                  <span className={`block truncate text-sm ${active ? "text-white/85" : "text-slate-500 dark:text-slate-400"}`}>
                    {busy[c.id] ? "Colibri is thinking..." : c.last_message || "No messages yet"}
                  </span>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </aside>
  );
}

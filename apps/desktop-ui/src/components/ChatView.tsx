import { useEffect, useRef } from "react";
import MessageBubble from "./MessageBubble";
import Composer from "./Composer";
import { TrashIcon } from "./Icons";
import { useStore } from "../lib/store";

export default function ChatView({ id }: { id: string }) {
  const messages = useStore((s) => s.messages[id]) ?? [];
  const busy = useStore((s) => s.busy[id] ?? false);
  const settings = useStore((s) => s.settings);
  const title = useStore((s) => [...s.drafts, ...s.conversations].find((c) => c.id === id)?.title) || "New chat";
  const remove = useStore((s) => s.deleteConversation);
  const scroller = useRef<HTMLDivElement>(null);
  const last = messages[messages.length - 1];

  useEffect(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages.length, last?.content, last?.tools.length, last?.pending]);

  const subtitle = busy
    ? "generating"
    : settings
      ? `${settings.colibri.model} on Colibri`
      : "Colibri";

  return (
    <section className="flex min-w-0 flex-1 flex-col">
      <header className="flex h-14 shrink-0 items-center justify-between border-b border-slate-200 bg-white px-4 dark:border-black/40 dark:bg-panel-dark">
        <div className="min-w-0">
          <div className="truncate font-semibold">{title}</div>
          <div className={`text-xs ${busy ? "text-accent" : "text-slate-400"}`}>{subtitle}</div>
        </div>
        <button
          onClick={() => {
            if (window.confirm("Delete this chat and its memory?")) remove(id);
          }}
          title="Delete chat"
          className="rounded-full p-2 text-slate-400 hover:bg-slate-100 hover:text-red-500 dark:hover:bg-panel-2-dark"
        >
          <TrashIcon width={18} height={18} />
        </button>
      </header>
      <div ref={scroller} className="chat-pattern scroll-thin flex-1 overflow-y-auto px-4 py-4">
        <div className="mx-auto flex max-w-3xl flex-col gap-1.5">
          {messages.length === 0 && (
            <div className="mx-auto mt-16 max-w-sm rounded-2xl bg-black/25 px-5 py-4 text-center text-sm text-white">
              Colibri streams experts from NVMe, so the first answer can take a while. Short questions answer fastest.
            </div>
          )}
          {messages.map((m) => (
            <MessageBubble key={m.key} message={m} />
          ))}
        </div>
      </div>
      <Composer busy={busy} />
    </section>
  );
}

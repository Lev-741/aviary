import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { SendIcon, StopIcon } from "./Icons";
import { useStore } from "../lib/store";

export default function Composer({ busy }: { busy: boolean }) {
  const [text, setText] = useState("");
  const send = useStore((s) => s.send);
  const cancel = useStore((s) => s.cancel);
  const activeId = useStore((s) => s.activeId);
  const area = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    area.current?.focus();
  }, [activeId]);

  useEffect(() => {
    const el = area.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [text]);

  const submit = () => {
    if (busy || !text.trim()) return;
    send(text);
    setText("");
  };

  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <div className="flex shrink-0 items-end gap-2 border-t border-slate-200 bg-white px-4 py-3 dark:border-black/40 dark:bg-panel-dark">
      <textarea
        ref={area}
        rows={1}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKey}
        placeholder="Write a message..."
        className="max-h-[200px] flex-1 resize-none bg-transparent py-2 outline-none placeholder:text-slate-400"
      />
      {busy ? (
        <button
          onClick={cancel}
          title="Stop"
          className="flex h-10 w-10 items-center justify-center rounded-full bg-red-500 text-white hover:bg-red-600"
        >
          <StopIcon width={18} height={18} />
        </button>
      ) : (
        <button
          onClick={submit}
          disabled={!text.trim()}
          title="Send"
          className="flex h-10 w-10 items-center justify-center rounded-full bg-accent text-white hover:bg-accent-dark disabled:opacity-40"
        >
          <SendIcon width={18} height={18} />
        </button>
      )}
    </div>
  );
}

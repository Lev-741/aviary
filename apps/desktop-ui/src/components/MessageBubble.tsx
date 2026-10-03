import { useState } from "react";
import { ToolIcon } from "./Icons";
import type { UiMessage } from "../lib/store";
import { clock, percent } from "../lib/format";

function ReplyMeta({ message }: { message: UiMessage }) {
  const reply = message.reply;
  if (!reply) return null;
  const parts = [
    `slot ${reply.slot} ${reply.warm_slot ? "warm" : "cold"}`,
    `${reply.usage.completion_tokens} tok`,
    `${(reply.elapsed_ms / 1000).toFixed(1)}s`,
  ];
  if (reply.usage.completion_tokens > 0 && reply.elapsed_ms > 0) {
    parts.push(`${((reply.usage.completion_tokens * 1000) / reply.elapsed_ms).toFixed(2)} tok/s`);
  }
  const routing = reply.routing;
  if (routing && routing.routed_last_turn > 0) {
    parts.push(`experts ${percent(routing.served_from_memory / routing.routed_last_turn)} in memory`);
  }
  return <div className="mt-1 text-[11px] text-slate-400">{parts.join("  |  ")}</div>;
}

export default function MessageBubble({ message }: { message: UiMessage }) {
  const [showReasoning, setShowReasoning] = useState(false);
  const outgoing = message.role === "user";
  const waiting = message.pending && !message.content && message.tools.length === 0;

  return (
    <div className={`flex ${outgoing ? "justify-end" : "justify-start"}`}>
      <div
        className={`relative max-w-[78%] rounded-2xl px-3 py-2 shadow-sm ${
          outgoing
            ? "rounded-br-md bg-bubble-out dark:bg-bubble-out-dark"
            : "rounded-bl-md bg-white dark:bg-panel-dark"
        }`}
      >
        {message.reasoning && (
          <button
            onClick={() => setShowReasoning((v) => !v)}
            className="mb-1 block text-xs text-accent hover:underline"
          >
            {showReasoning ? "Hide reasoning" : "Show reasoning"}
          </button>
        )}
        {showReasoning && (
          <div className="selectable mb-2 whitespace-pre-wrap border-l-2 border-accent/40 pl-2 text-sm text-slate-500 dark:text-slate-400">
            {message.reasoning}
          </div>
        )}
        {message.tools.map((tool, i) => (
          <div key={i} className="mb-1.5 rounded-lg bg-slate-100 px-2 py-1.5 text-xs dark:bg-panel-2-dark">
            <div className="flex items-center gap-1.5 font-medium text-slate-600 dark:text-slate-300">
              <ToolIcon width={13} height={13} />
              {tool.name}
              <span className="truncate font-mono font-normal text-slate-400">{tool.arguments}</span>
            </div>
            {tool.output !== undefined && (
              <pre
                className={`selectable mt-1 max-h-32 overflow-auto whitespace-pre-wrap font-mono ${
                  tool.ok ? "text-slate-500 dark:text-slate-400" : "text-red-500"
                }`}
              >
                {tool.output}
              </pre>
            )}
          </div>
        ))}
        {waiting ? (
          <div className="typing flex gap-1 py-1.5 text-slate-400">
            <span>&#9679;</span>
            <span>&#9679;</span>
            <span>&#9679;</span>
          </div>
        ) : (
          message.content && (
            <div className="selectable whitespace-pre-wrap break-words leading-snug">{message.content}</div>
          )
        )}
        {message.error && <div className="mt-1 text-sm text-red-500">{message.error}</div>}
        <div className="flex items-end justify-end gap-2">
          {!outgoing && <ReplyMeta message={message} />}
          <span className="mt-0.5 text-[11px] text-slate-400">{clock(message.createdAt)}</span>
        </div>
      </div>
    </div>
  );
}

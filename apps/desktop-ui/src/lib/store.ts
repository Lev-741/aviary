import { create } from "zustand";
import { api, errorText, type AgentReply, type ChatEnvelope, type Conversation, type Settings } from "./api";

export type Page = "chats" | "monitor" | "settings";

export interface ToolActivity {
  name: string;
  arguments: string;
  output?: string;
  ok?: boolean;
}

export interface UiMessage {
  key: string;
  role: "user" | "assistant";
  content: string;
  reasoning: string;
  createdAt: number;
  pending: boolean;
  error?: string;
  tools: ToolActivity[];
  reply?: AgentReply;
  slot?: { slot: number; warm: boolean };
}

interface State {
  page: Page;
  conversations: Conversation[];
  drafts: Conversation[];
  activeId: string | null;
  messages: Record<string, UiMessage[]>;
  busy: Record<string, boolean>;
  settings: Settings | null;
  setPage: (page: Page) => void;
  loadSettings: () => Promise<void>;
  setSettings: (settings: Settings) => void;
  refreshConversations: () => Promise<void>;
  newConversation: () => void;
  openConversation: (id: string) => Promise<void>;
  deleteConversation: (id: string) => Promise<void>;
  send: (text: string) => Promise<void>;
  cancel: () => Promise<void>;
  applyEvent: (event: ChatEnvelope) => void;
}

let counter = 0;
const key = () => `m${Date.now().toString(36)}${(counter++).toString(36)}`;

function updateLast(list: UiMessage[], change: (m: UiMessage) => UiMessage): UiMessage[] {
  const index = list.length - 1;
  if (index < 0 || list[index].role !== "assistant" || !list[index].pending) return list;
  const next = list.slice();
  next[index] = change(list[index]);
  return next;
}

export const useStore = create<State>((set, get) => ({
  page: "chats",
  conversations: [],
  drafts: [],
  activeId: null,
  messages: {},
  busy: {},
  settings: null,

  setPage: (page) => set({ page }),

  loadSettings: async () => {
    const settings = await api.getSettings();
    set({ settings });
  },

  setSettings: (settings) => set({ settings }),

  refreshConversations: async () => {
    const conversations = await api.listConversations();
    const known = new Set(conversations.map((c) => c.id));
    set((s) => ({ conversations, drafts: s.drafts.filter((d) => !known.has(d.id)) }));
    if (!get().activeId && conversations.length > 0) {
      await get().openConversation(conversations[0].id);
    }
  },

  newConversation: () => {
    const id = `desktop:${Date.now().toString(36)}`;
    const draft: Conversation = { id, title: "", last_message: "", updated_at: Math.floor(Date.now() / 1000) };
    set((s) => ({
      drafts: [draft, ...s.drafts],
      activeId: id,
      messages: { ...s.messages, [id]: [] },
      page: "chats",
    }));
  },

  openConversation: async (id) => {
    set({ activeId: id, page: "chats" });
    if (get().busy[id] || get().drafts.some((d) => d.id === id)) return;
    const history = await api.getHistory(id);
    const messages: UiMessage[] = history
      .filter((m) => m.role === "user" || m.role === "assistant")
      .map((m) => ({
        key: `db${m.id}`,
        role: m.role as "user" | "assistant",
        content: m.content,
        reasoning: "",
        createdAt: m.created_at * 1000,
        pending: false,
        tools: [],
      }));
    set((s) => ({ messages: { ...s.messages, [id]: messages } }));
  },

  deleteConversation: async (id) => {
    await api.deleteConversation(id);
    set((s) => {
      const messages = { ...s.messages };
      delete messages[id];
      return {
        messages,
        drafts: s.drafts.filter((d) => d.id !== id),
        conversations: s.conversations.filter((c) => c.id !== id),
        activeId: s.activeId === id ? null : s.activeId,
      };
    });
    await get().refreshConversations();
  },

  send: async (text) => {
    const id = get().activeId;
    const trimmed = text.trim();
    if (!id || !trimmed || get().busy[id]) return;
    const now = Date.now();
    const user: UiMessage = { key: key(), role: "user", content: trimmed, reasoning: "", createdAt: now, pending: false, tools: [] };
    const assistant: UiMessage = { key: key(), role: "assistant", content: "", reasoning: "", createdAt: now, pending: true, tools: [] };
    set((s) => ({
      busy: { ...s.busy, [id]: true },
      messages: { ...s.messages, [id]: [...(s.messages[id] ?? []), user, assistant] },
      drafts: s.drafts.map((d) => (d.id === id && !d.title ? { ...d, title: trimmed } : d)),
    }));
    try {
      const reply = await api.sendMessage(id, trimmed);
      set((s) => ({
        messages: {
          ...s.messages,
          [id]: updateLast(s.messages[id] ?? [], (m) => ({
            ...m,
            content: reply.content || m.content,
            reasoning: reply.reasoning ?? m.reasoning,
            pending: false,
            reply,
            createdAt: Date.now(),
          })),
        },
      }));
    } catch (error) {
      set((s) => ({
        messages: {
          ...s.messages,
          [id]: updateLast(s.messages[id] ?? [], (m) => ({ ...m, pending: false, error: errorText(error) })),
        },
      }));
    } finally {
      set((s) => ({ busy: { ...s.busy, [id]: false } }));
      get().refreshConversations().catch(() => undefined);
    }
  },

  cancel: async () => {
    const id = get().activeId;
    if (id) await api.cancelMessage(id);
  },

  applyEvent: (event) => {
    const id = event.conversation;
    set((s) => {
      const list = s.messages[id];
      if (!list) return s;
      const next = updateLast(list, (m) => {
        switch (event.kind) {
          case "started":
            return { ...m, slot: { slot: event.slot, warm: event.warm } };
          case "token":
            return { ...m, content: m.content + event.text };
          case "reasoning":
            return { ...m, reasoning: m.reasoning + event.text };
          case "tool_call":
            return { ...m, content: "", tools: [...m.tools, { name: event.name, arguments: event.arguments }] };
          case "tool_result": {
            const tools = m.tools.slice();
            const index = tools.map((t) => t.name).lastIndexOf(event.name);
            if (index >= 0) tools[index] = { ...tools[index], output: event.output, ok: event.ok };
            return { ...m, tools };
          }
        }
      });
      return { messages: { ...s.messages, [id]: next } };
    });
  },
}));

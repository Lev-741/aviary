import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Role = "system" | "user" | "assistant" | "tool";

export interface ColibriConfig {
  base_url: string;
  model: string;
  api_key: string | null;
  kv_slots: number;
  request_timeout_secs: number;
  max_tokens: number;
  context_tokens: number;
  temperature: number | null;
  thinking: boolean;
}

export interface Settings {
  colibri: ColibriConfig;
  instructions: string;
  tools_enabled: boolean;
  workspace: string | null;
  allow_write: boolean;
  allow_shell: boolean;
}

export interface Scheduler {
  active: number;
  queued: number;
  capacity: number;
  max_queue: number;
  queue_timeout_seconds: number;
  admitted: number;
  completed: number;
  failed: number;
  rejected: number;
  timed_out: number;
  cancelled: number;
}

export interface Tiers {
  vram: number;
  ram: number;
  disk: number;
  vram_gb: number;
  ram_gb: number;
}

export interface HwInfo {
  cores: number;
  ram_total_gb: number;
  ram_avail_gb: number;
  gpus: number;
  vram_total_gb: number;
  cpu: string;
  gpu: string;
}

export interface Health {
  status: string;
  scheduler: Scheduler | null;
  kv_slots: number | null;
  tiers: Tiers | null;
  hwinfo: HwInfo | null;
}

export interface Residency {
  disk: number;
  ram: number;
  vram: number;
}

export interface RoutingStats {
  residency: Residency;
  routed_last_turn: number;
  served_from_memory: number;
  streamed_from_disk: number;
}

export interface ProfileTurn {
  wall_s: number;
  prompt_tokens: number;
  completion_tokens: number;
  expert_disk_s: number;
  expert_wait_s: number;
  expert_matmul_s: number;
  attention_s: number;
  lm_head_s: number;
  forwards: number;
}

export interface NvmeStatus {
  experts_on_disk: number;
  disk_ratio: number;
  last_turn_disk_s: number;
  last_turn_io_wait_share: number;
  last_turn_streamed_experts: number;
}

export type StreamingState = "idle" | "generating" | "queued" | "unknown";

export interface ModelInfo {
  id: string;
  owned_by: string | null;
  created: number | null;
}

export interface ColibriStatus {
  reachable: boolean;
  base_url: string;
  configured_model: string;
  models: ModelInfo[];
  health: Health | null;
  routing: RoutingStats | null;
  last_turn: ProfileTurn | null;
  recent_turns: ProfileTurn[];
  state: StreamingState;
  nvme: NvmeStatus;
  error: string | null;
}

export interface SlotInfo {
  slot: number;
  owner: string | null;
  turns: number;
  idle_secs: number | null;
  last_hit_rate: number | null;
}

export interface CacheStats {
  slots: SlotInfo[];
  warm_turns: number;
  cold_turns: number;
}

export interface StatusPayload {
  status: ColibriStatus;
  cache: CacheStats;
}

export interface ExpertSnapshot {
  rows: number;
  cols: number;
  map: string;
  hits: string;
  seq: number;
}

export interface Conversation {
  id: string;
  title: string;
  last_message: string;
  updated_at: number;
}

export interface StoredMessage {
  id: number;
  role: Role;
  content: string;
  created_at: number;
}

export interface Usage {
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
}

export interface AgentReply {
  content: string;
  reasoning: string | null;
  usage: Usage;
  tool_calls: number;
  slot: number;
  warm_slot: boolean;
  routing: RoutingStats | null;
  elapsed_ms: number;
  queue_wait_ms: number | null;
}

export type ChatEvent =
  | { kind: "started"; slot: number; warm: boolean; trimmed: number }
  | { kind: "token"; text: string }
  | { kind: "reasoning"; text: string }
  | { kind: "tool_call"; name: string; arguments: string }
  | { kind: "tool_result"; name: string; output: string; ok: boolean };

export type ChatEnvelope = ChatEvent & { conversation: string };

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  probeModels: (baseUrl: string, apiKey: string | null) =>
    invoke<string[]>("probe_models", { baseUrl, apiKey }),
  getStatus: () => invoke<StatusPayload>("get_status"),
  getExperts: () => invoke<ExpertSnapshot>("get_experts"),
  listConversations: () => invoke<Conversation[]>("list_conversations"),
  getHistory: (conversation: string) => invoke<StoredMessage[]>("get_history", { conversation }),
  deleteConversation: (conversation: string) => invoke<number>("delete_conversation", { conversation }),
  sendMessage: (conversation: string, text: string) =>
    invoke<AgentReply>("send_message", { conversation, text }),
  cancelMessage: (conversation: string) => invoke<boolean>("cancel_message", { conversation }),
};

export function onChatEvent(handler: (event: ChatEnvelope) => void): Promise<UnlistenFn> {
  return listen<ChatEnvelope>("aviary://chat", (event) => handler(event.payload));
}

export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return JSON.stringify(error);
}

import { useEffect, useState, type ReactNode } from "react";
import { api, errorText, type Settings } from "../lib/api";
import { useStore } from "../lib/store";

function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1 block text-sm font-medium">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-xs text-slate-400">{hint}</span>}
    </label>
  );
}

function Toggle({ label, hint, checked, onChange }: { label: string; hint?: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="flex cursor-pointer items-start gap-3">
      <input type="checkbox" checked={checked} onChange={(e) => onChange(e.target.checked)} className="mt-1 h-4 w-4 accent-accent" />
      <span>
        <span className="block text-sm font-medium">{label}</span>
        {hint && <span className="block text-xs text-slate-400">{hint}</span>}
      </span>
    </label>
  );
}

const input =
  "w-full rounded-lg border border-slate-200 bg-white px-3 py-2 text-sm outline-none focus:border-accent dark:border-panel-2-dark dark:bg-panel-2-dark";

export default function SettingsPage() {
  const stored = useStore((s) => s.settings);
  const setStored = useStore((s) => s.setSettings);
  const [draft, setDraft] = useState<Settings | null>(stored);
  const [models, setModels] = useState<string[]>([]);
  const [probe, setProbe] = useState<{ ok: boolean; text: string } | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);

  useEffect(() => {
    if (stored && !draft) setDraft(stored);
  }, [stored, draft]);

  if (!draft) {
    return <div className="flex flex-1 items-center justify-center text-slate-400">Loading settings...</div>;
  }

  const colibri = draft.colibri;
  const update = (patch: Partial<Settings>) => setDraft({ ...draft, ...patch });
  const updateColibri = (patch: Partial<Settings["colibri"]>) => setDraft({ ...draft, colibri: { ...colibri, ...patch } });
  const numeric = (value: string, fallback: number) => {
    const n = Number(value);
    return Number.isFinite(n) && value.trim() !== "" ? n : fallback;
  };

  const testConnection = async () => {
    setProbe(null);
    try {
      const list = await api.probeModels(colibri.base_url, colibri.api_key);
      setModels(list);
      setProbe({ ok: true, text: `Connected. Serving: ${list.join(", ") || "no models"}` });
    } catch (error) {
      setProbe({ ok: false, text: errorText(error) });
    }
  };

  const save = async () => {
    setSaving(true);
    setSaved(null);
    try {
      const result = await api.saveSettings(draft);
      setStored(result);
      setDraft(result);
      setSaved("Saved. New messages use these settings.");
    } catch (error) {
      setSaved(`Not saved: ${errorText(error)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="scroll-thin flex-1 overflow-y-auto bg-slate-50 p-6 dark:bg-chat-bg-dark">
      <div className="mx-auto flex max-w-2xl flex-col gap-4">
        <h1 className="text-xl font-semibold">Settings</h1>

        <section className="flex flex-col gap-4 rounded-xl bg-white p-5 shadow-sm dark:bg-panel-dark">
          <h2 className="text-xs font-semibold uppercase tracking-wide text-slate-400">Colibri server</h2>
          <Field label="API URL" hint="The address of coli serve, including /v1.">
            <input className={input} value={colibri.base_url} onChange={(e) => updateColibri({ base_url: e.target.value })} />
          </Field>
          <Field label="API key" hint="Only needed when Colibri runs with COLI_API_KEY. Also unlocks hardware and expert telemetry.">
            <input
              className={input}
              type="password"
              value={colibri.api_key ?? ""}
              onChange={(e) => updateColibri({ api_key: e.target.value || null })}
            />
          </Field>
          <div className="flex items-center gap-3">
            <button onClick={testConnection} className="rounded-lg border border-accent px-3 py-1.5 text-sm text-accent hover:bg-accent/10">
              Test connection
            </button>
            {probe && <span className={`text-sm ${probe.ok ? "text-emerald-600" : "text-red-500"}`}>{probe.text}</span>}
          </div>
          <Field label="Model" hint="The --model-id Colibri was started with.">
            <input className={input} list="colibri-models" value={colibri.model} onChange={(e) => updateColibri({ model: e.target.value })} />
            <datalist id="colibri-models">
              {models.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
          </Field>
          <div className="grid grid-cols-3 gap-4">
            <Field label="KV slots" hint="Match coli serve --kv-slots.">
              <input
                className={input}
                type="number"
                min={1}
                max={16}
                value={colibri.kv_slots}
                onChange={(e) => updateColibri({ kv_slots: numeric(e.target.value, colibri.kv_slots) })}
              />
            </Field>
            <Field label="Context tokens" hint="Colibri KV context size.">
              <input
                className={input}
                type="number"
                min={512}
                value={colibri.context_tokens}
                onChange={(e) => updateColibri({ context_tokens: numeric(e.target.value, colibri.context_tokens) })}
              />
            </Field>
            <Field label="Max reply tokens">
              <input
                className={input}
                type="number"
                min={16}
                value={colibri.max_tokens}
                onChange={(e) => updateColibri({ max_tokens: numeric(e.target.value, colibri.max_tokens) })}
              />
            </Field>
          </div>
          <div className="grid grid-cols-2 gap-4">
            <Field label="Temperature" hint="Empty uses the server default.">
              <input
                className={input}
                type="number"
                step={0.1}
                min={0}
                max={2}
                value={colibri.temperature ?? ""}
                onChange={(e) => updateColibri({ temperature: e.target.value === "" ? null : numeric(e.target.value, 0.7) })}
              />
            </Field>
            <Field label="Request timeout (s)" hint="Long prefills on NVMe can take minutes.">
              <input
                className={input}
                type="number"
                min={30}
                value={colibri.request_timeout_secs}
                onChange={(e) => updateColibri({ request_timeout_secs: numeric(e.target.value, colibri.request_timeout_secs) })}
              />
            </Field>
          </div>
          <Toggle
            label="Reasoning mode"
            hint="Lets the model think before answering. Better answers, many more generated tokens."
            checked={colibri.thinking}
            onChange={(thinking) => updateColibri({ thinking })}
          />
        </section>

        <section className="flex flex-col gap-4 rounded-xl bg-white p-5 shadow-sm dark:bg-panel-dark">
          <h2 className="text-xs font-semibold uppercase tracking-wide text-slate-400">Assistant</h2>
          <Field label="Extra instructions" hint="Appended to the system prompt. Keep it short: every token is prefilled from NVMe.">
            <textarea className={`${input} min-h-20`} value={draft.instructions} onChange={(e) => update({ instructions: e.target.value })} />
          </Field>
          <Toggle
            label="Tools"
            hint="Let the model read files and list directories in the workspace."
            checked={draft.tools_enabled}
            onChange={(tools_enabled) => update({ tools_enabled })}
          />
          <Field label="Workspace">
            <input className={input} value={draft.workspace ?? ""} onChange={(e) => update({ workspace: e.target.value || null })} />
          </Field>
          <Toggle label="Allow writing files" checked={draft.allow_write} onChange={(allow_write) => update({ allow_write })} />
          <Toggle
            label="Allow shell commands"
            hint="Runs commands in the workspace without asking. Only enable for trusted use."
            checked={draft.allow_shell}
            onChange={(allow_shell) => update({ allow_shell })}
          />
        </section>

        <div className="flex items-center gap-3">
          <button
            onClick={save}
            disabled={saving}
            className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white hover:bg-accent-dark disabled:opacity-50"
          >
            {saving ? "Saving..." : "Save"}
          </button>
          {saved && <span className="text-sm text-slate-500">{saved}</span>}
        </div>
      </div>
    </div>
  );
}

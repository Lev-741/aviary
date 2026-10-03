export const percent = (value: number) => `${Math.round(value * 100)}%`;

export const gb = (value: number) => `${value.toFixed(1)} GB`;

export const count = (value: number) => value.toLocaleString("en-US");

export const seconds = (value: number) => (value >= 100 ? `${Math.round(value)}s` : `${value.toFixed(1)}s`);

export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function shortDate(unixSeconds: number): string {
  const date = new Date(unixSeconds * 1000);
  const now = new Date();
  if (date.toDateString() === now.toDateString()) return clock(date.getTime());
  const days = (now.getTime() - date.getTime()) / 86_400_000;
  if (days < 7) return date.toLocaleDateString([], { weekday: "short" });
  return date.toLocaleDateString([], { day: "2-digit", month: "2-digit", year: "2-digit" });
}

export function initials(title: string): string {
  const words = title.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "N";
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return (words[0][0] + words[1][0]).toUpperCase();
}

const palette = ["#e17076", "#7bc862", "#e5ca77", "#65aadd", "#a695e7", "#ee7aae", "#6ec9cb", "#faa774"];

export function avatarColor(id: string): string {
  let hash = 0;
  for (let i = 0; i < id.length; i++) hash = (hash * 31 + id.charCodeAt(i)) | 0;
  return palette[Math.abs(hash) % palette.length];
}

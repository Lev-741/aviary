import { useEffect } from "react";
import NavRail from "./components/NavRail";
import ChatPage from "./pages/ChatPage";
import MonitorPage from "./pages/MonitorPage";
import SettingsPage from "./pages/SettingsPage";
import { onChatEvent } from "./lib/api";
import { useStore } from "./lib/store";

export default function App() {
  const page = useStore((s) => s.page);

  useEffect(() => {
    const { applyEvent, loadSettings, refreshConversations } = useStore.getState();
    const unlisten = onChatEvent(applyEvent);
    loadSettings().catch(() => undefined);
    refreshConversations().catch(() => undefined);
    return () => {
      unlisten.then((fn) => fn()).catch(() => undefined);
    };
  }, []);

  return (
    <div className="flex h-full text-[15px] text-slate-900 dark:text-slate-100">
      <NavRail />
      <main className="flex min-w-0 flex-1">
        {page === "chats" && <ChatPage />}
        {page === "monitor" && <MonitorPage />}
        {page === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}

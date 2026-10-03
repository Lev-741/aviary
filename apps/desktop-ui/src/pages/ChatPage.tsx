import ConversationList from "../components/ConversationList";
import ChatView from "../components/ChatView";
import { useStore } from "../lib/store";

export default function ChatPage() {
  const activeId = useStore((s) => s.activeId);
  const newConversation = useStore((s) => s.newConversation);

  return (
    <div className="flex min-w-0 flex-1">
      <ConversationList />
      {activeId ? (
        <ChatView id={activeId} />
      ) : (
        <div className="chat-pattern flex flex-1 items-center justify-center">
          <button
            onClick={newConversation}
            className="rounded-full bg-black/25 px-4 py-1.5 text-sm text-white hover:bg-black/35"
          >
            Select a chat or start a new one
          </button>
        </div>
      )}
    </div>
  );
}

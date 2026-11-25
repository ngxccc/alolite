import { useEffect, useRef } from "react";
import { ipc, isTauriRuntime } from "@/shared/api/ipc";
import type { Thread } from "@/shared/types/ipc.generated";
import { cn } from "cn";
import { dayKey, matchesVnQuery } from "@/shared/utils/text";
import { ChatMessageList } from "@/shared/ui/chat";
import { useChatStore, useContactsStore, useUIStore } from "@/stores";
import { Composer } from "./composer";
import { ChatHeader } from "./chat-header";
import { PinnedBar } from "./pinned-bar";
import { DateDivider } from "./date-divider";
import { ChatHistoryBanners } from "./chat-history-banners";
import { MessageBubble } from "./message-bubble";
import { useChatScroll } from "./use-chat-scroll";
import { useSenderLookup } from "./use-sender-lookup";

export function ChatArea() {
  const selectedThreadId = useChatStore((s) => s.selectedThreadId);
  const threads = useChatStore((s) => s.threads);
  const filterStarredOnly = useChatStore((s) => s.filterStarredOnly);
  const renderLimitByThread = useChatStore((s) => s.renderLimitByThread);
  const historyReachedStart = useChatStore((s) => s.historyReachedStart);
  const quoteHighlightId = useChatStore((s) => s.quoteHighlightId);
  const toggleInfoDrawer = useChatStore((s) => s.toggleInfoDrawer);
  const inChatSearchOpen = useUIStore((s) => s.inChatSearchOpen);
  const inChatSearchQuery = useUIStore((s) => s.inChatSearchQuery);
  const friendStatusByUid = useContactsStore((s) => s.friendStatusByUid);
  const hydratedThreadIdsRef = useRef<Set<string>>(new Set());

  const thread: Thread | undefined = threads.find((t) => t.id === selectedThreadId);
  const pinnedMessages = thread?.pinned_messages.length
    ? thread.pinned_messages
    : (thread?.messages.filter((m) => m.is_pinned) ?? []);

  const { resolveSender, resolveReplySenderName } = useSenderLookup();

  let visibleMessages = thread?.messages ?? [];
  if (filterStarredOnly) {
    visibleMessages = visibleMessages.filter((m) => Boolean(m.is_starred));
  }
  if (inChatSearchOpen && inChatSearchQuery.trim()) {
    visibleMessages = visibleMessages.filter((m) => matchesVnQuery(m.text, inChatSearchQuery));
  }
  const limit = thread ? (renderLimitByThread[thread.id] ?? 150) : 150;
  if (visibleMessages.length > limit) {
    visibleMessages = visibleMessages.slice(-limit);
  }

  useEffect(() => {
    if (!thread || !isTauriRuntime()) return;
    if (!hydratedThreadIdsRef.current.has(thread.id)) {
      hydratedThreadIdsRef.current.add(thread.id);
      void ipc.loadGroupHistory(thread.id, thread.is_group, thread.messages.length);
    }
    if (
      !thread.is_group &&
      !thread.name.includes("Cloud của tôi") &&
      !friendStatusByUid[thread.id]
    ) {
      void ipc.checkFriendStatus(thread.id);
    }
  }, [thread, friendStatusByUid]);

  const { scrollRef, msgRefs, jumpToQuoted } = useChatScroll({
    thread,
    visibleMessagesCount: visibleMessages.length,
    quoteHighlightId,
  });

  if (!thread) {
    return (
      <main className="grid flex-1 place-items-center bg-canvas p-6">
        <div className="max-w-sm text-center text-muted-foreground">
          <div className="mb-1.5 text-lg font-bold text-foreground">Chào mừng đến với AloLite</div>
          <div className="text-sm">
            Chọn một hội thoại bên trái để bắt đầu trò chuyện hoặc tra cứu số điện thoại mới.
          </div>
        </div>
      </main>
    );
  }

  const totalCount = thread.messages.length;
  const renderLimit = renderLimitByThread[thread.id] ?? 150;
  const canShowMoreLocal = totalCount > renderLimit;
  const reachedStart = Boolean(historyReachedStart[thread.id]);

  let prevDay = "";

  return (
    <main className="flex min-w-0 flex-1 flex-col bg-canvas">
      {/* ── Chat Header ── 68px per spec ── */}
      <ChatHeader thread={thread} />

      {/* ── Pinned message bar ── */}
      <PinnedBar
        pinnedMessages={pinnedMessages}
        onJumpToPinned={jumpToQuoted}
        onToggleInfoDrawer={toggleInfoDrawer}
      />

      {/* ── Message list (shadcn-chat ChatMessageList) ── */}
      <ChatMessageList ref={scrollRef}>
        {/* Load history banners — styled as quiet chips */}
        <ChatHistoryBanners
          thread={thread}
          canShowMoreLocal={canShowMoreLocal}
          reachedStart={reachedStart}
        />

        {visibleMessages.map((msg, idx) => {
          const dk = dayKey(msg.ts);
          const showDivider = Boolean(dk && dk !== prevDay);
          if (dk) prevDay = dk;

          const prevMsg = !showDivider && idx > 0 ? visibleMessages.at(idx - 1) : undefined;
          const isGroupedWithPrev =
            prevMsg?.is_outgoing === msg.is_outgoing && prevMsg.sender === msg.sender;

          const msgKey = msg.id || `${msg.ts}_${idx}`;
          const highlighted = quoteHighlightId === msgKey;
          const senderInfo = !msg.is_outgoing ? resolveSender(msg, thread) : null;

          return (
            <div
              key={msgKey}
              className={cn("flex flex-col", isGroupedWithPrev ? "mt-0.5" : "mt-2")}
            >
              {/* Date divider chip */}
              {showDivider ? <DateDivider ts={msg.ts} /> : null}

              {/* Message row */}
              <MessageBubble
                msg={msg}
                thread={thread}
                isGroupedWithPrev={isGroupedWithPrev}
                highlighted={highlighted}
                senderInfo={senderInfo}
                jumpToQuoted={jumpToQuoted}
                resolveReplySenderName={resolveReplySenderName}
                setRef={(el) => {
                  msgRefs.current[msgKey] = el;
                }}
              />
            </div>
          );
        })}
      </ChatMessageList>

      <Composer thread={thread} />
    </main>
  );
}

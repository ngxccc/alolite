import { useEffect, useRef } from "react";
import type { ChatMessage, Thread } from "@/shared/types/ipc.generated";
import { uniqueReferenceIndex } from "@/features/chat/utils/messages";
import { useChatStore, useUIStore } from "@/stores";

interface UseChatScrollOptions {
  thread?: Thread;
  visibleMessagesCount: number;
  quoteHighlightId: string | null;
}

export function useChatScroll({
  thread,
  visibleMessagesCount,
  quoteHighlightId,
}: UseChatScrollOptions) {
  const setQuoteHighlightId = useChatStore((s) => s.setQuoteHighlightId);
  const showToast = useUIStore((s) => s.showToast);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const msgRefs = useRef<Record<string, HTMLDivElement | null>>({});

  useEffect(() => {
    if (scrollRef.current && !quoteHighlightId) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [thread?.id, visibleMessagesCount, quoteHighlightId]);

  const jumpToQuoted = (msg: ChatMessage) => {
    if (!thread) return;
    const ref = msg.reply_to_id ?? msg.reply_to_text ?? "";
    const idx = uniqueReferenceIndex(thread.messages, ref);
    if (idx === null) {
      showToast("Không tìm thấy tin nhắn gốc duy nhất trong lịch sử đang tải");
      return;
    }
    const target = thread.messages[idx];
    if (!target) return;
    const targetKey = target.id || String(target.ts);
    setQuoteHighlightId(targetKey);
    requestAnimationFrame(() => {
      msgRefs.current[targetKey]?.scrollIntoView({
        behavior: "smooth",
        block: "center",
      });
    });
    window.setTimeout(() => setQuoteHighlightId(null), 2200);
  };

  return {
    scrollRef,
    msgRefs,
    jumpToQuoted,
  };
}

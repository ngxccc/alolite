import { ChevronUp } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import type { Thread } from "@/shared/types/ipc.generated";
import { useChatStore } from "@/stores";

interface ChatHistoryBannersProps {
  thread: Thread;
  canShowMoreLocal: boolean;
  reachedStart: boolean;
}

export function ChatHistoryBanners({
  thread,
  canShowMoreLocal,
  reachedStart,
}: ChatHistoryBannersProps) {
  const expandRenderLimit = useChatStore((s) => s.expandRenderLimit);
  if (!canShowMoreLocal && reachedStart) return null;

  return (
    <div className="mb-3 flex flex-col items-center gap-2">
      {canShowMoreLocal ? (
        <button
          type="button"
          className="flex cursor-pointer items-center gap-1.5 self-center rounded-full bg-[rgba(255,255,255,0.05)] px-4 py-1 text-xs text-muted-foreground transition hover:bg-[rgba(255,255,255,0.09)] active:scale-95"
          onClick={() => expandRenderLimit(thread.id, 150)}
        >
          <ChevronUp className="h-3 w-3" />
          Xem thêm tin cũ trong máy
        </button>
      ) : null}
      {!reachedStart && (thread.is_group || thread.name.includes("Cloud của tôi")) ? (
        <button
          type="button"
          className="flex cursor-pointer items-center gap-1.5 self-center rounded-full bg-[rgba(255,255,255,0.05)] px-4 py-1 text-xs text-muted-foreground transition hover:bg-[rgba(255,255,255,0.09)] active:scale-95"
          onClick={() =>
            void ipc.loadGroupHistory(thread.id, thread.is_group, thread.messages.length)
          }
        >
          <ChevronUp className="h-3 w-3" />
          Tải lịch sử cũ từ máy chủ
        </button>
      ) : null}
    </div>
  );
}

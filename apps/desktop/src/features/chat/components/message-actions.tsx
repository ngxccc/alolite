import { CornerUpLeft, Forward, RotateCcw, Star, Trash2 } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import type { ChatMessage, Thread } from "@/shared/types/ipc.generated";
import {
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
} from "@/shared/ui/context-menu";
import { cn } from "cn";
import { useChatStore, useUIStore } from "@/stores";
import { REACTION_LIST } from "../constants/emojis";
import { ReactionBadge } from "./message-reactions";

interface MessageContextMenuProps {
  msg: ChatMessage;
  thread: Thread;
  onSelectReaction: (emoji: string) => void;
}

export function MessageContextMenu({ msg, thread, onSelectReaction }: MessageContextMenuProps) {
  const setReplyingTo = useChatStore((s) => s.setReplyingTo);
  const setForwardMsg = useChatStore((s) => s.setForwardMsg);
  const toggleLocalStar = useChatStore((s) => s.toggleLocalStar);
  const deleteMessageLocal = useChatStore((s) => s.deleteMessageLocal);
  const setStatusMessage = useUIStore((s) => s.setStatusMessage);

  return (
    <ContextMenuContent>
      <div className="flex items-center gap-1 px-1.5 py-1">
        {REACTION_LIST.map((r) => (
          <button
            key={r.key}
            type="button"
            title={r.label}
            onClick={() => onSelectReaction(r.emoji)}
            className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-md transition-transform hover:scale-120 hover:bg-accent"
          >
            <ReactionBadge raw={r.emoji} />
          </button>
        ))}
      </div>
      <ContextMenuSeparator />
      <ContextMenuItem onClick={() => setReplyingTo(msg)}>
        <CornerUpLeft className="h-4 w-4" /> Trả lời
      </ContextMenuItem>
      <ContextMenuItem onClick={() => setForwardMsg(msg)}>
        <Forward className="h-4 w-4" /> Chuyển tiếp
      </ContextMenuItem>
      <ContextMenuItem onClick={() => toggleLocalStar(thread.id, msg.id)}>
        <Star className={cn("h-4 w-4", msg.is_starred && "text-warning")} />
        {msg.is_starred ? "Bỏ gắn sao" : "Gắn sao"}
      </ContextMenuItem>
      {msg.is_outgoing ? (
        <ContextMenuItem onClick={() => void ipc.recallMessage(thread.id, thread.is_group, msg)}>
          <RotateCcw className="h-4 w-4" /> Thu hồi tin nhắn
        </ContextMenuItem>
      ) : null}
      <ContextMenuSeparator />
      <ContextMenuItem
        variant="destructive"
        onClick={() => {
          deleteMessageLocal(thread.id, msg.id, msg.ts);
          setStatusMessage("Đã xóa tin nhắn khỏi lịch sử máy này");
          void ipc.deleteForMe(thread.id, thread.is_group, msg);
        }}
      >
        <Trash2 className="h-4 w-4" /> Xóa chỉ ở phía tôi
      </ContextMenuItem>
    </ContextMenuContent>
  );
}

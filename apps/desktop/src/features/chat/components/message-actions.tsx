import { CornerUpLeft, Forward, RotateCcw, SmilePlus, Star, Trash2 } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import type { ChatMessage, Thread } from "@/shared/types/ipc.generated";
import { Popover, PopoverTrigger } from "@/shared/ui/popover";
import { ChatBubbleAction, ChatBubbleActionWrapper } from "@/shared/ui/chat";
import { cn } from "cn";
import { useChatStore, useUIStore } from "@/stores";
import { ReactionPickerContent } from "./message-reactions";

interface MessageActionsProps {
  msg: ChatMessage;
  msgKey: string;
  thread: Thread;
  reactPickerFor: string | null;
  setReactPickerFor: (key: string | null) => void;
  onSelectReaction: (emoji: string) => void;
  variant?: "sent" | "received";
}

export function MessageActions({
  msg,
  msgKey,
  thread,
  reactPickerFor,
  setReactPickerFor,
  onSelectReaction,
  variant,
}: MessageActionsProps) {
  const setReplyingTo = useChatStore((s) => s.setReplyingTo);
  const setForwardMsg = useChatStore((s) => s.setForwardMsg);
  const toggleLocalStar = useChatStore((s) => s.toggleLocalStar);
  const deleteMessageLocal = useChatStore((s) => s.deleteMessageLocal);
  const setStatusMessage = useUIStore((s) => s.setStatusMessage);
  const isPickerOpen = reactPickerFor === msgKey;

  return (
    <ChatBubbleActionWrapper variant={variant} forceVisible={isPickerOpen}>
      <ChatBubbleAction
        icon={<CornerUpLeft className="h-3.5 w-3.5" />}
        title="Trả lời"
        onClick={() => setReplyingTo(msg)}
      />

      <Popover open={isPickerOpen} onOpenChange={(open) => setReactPickerFor(open ? msgKey : null)}>
        <PopoverTrigger
          render={
            <ChatBubbleAction icon={<SmilePlus className="h-3.5 w-3.5" />} title="Thả cảm xúc" />
          }
        />
        <ReactionPickerContent onSelectReaction={onSelectReaction} />
      </Popover>

      <ChatBubbleAction
        icon={<Forward className="h-3.5 w-3.5" />}
        title="Chuyển tiếp"
        onClick={() => setForwardMsg(msg)}
      />

      <ChatBubbleAction
        icon={<Star className={cn("h-3.5 w-3.5", msg.is_starred && "text-warning")} />}
        title="Gắn sao"
        onClick={() => toggleLocalStar(thread.id, msg.id)}
      />

      {msg.is_outgoing ? (
        <ChatBubbleAction
          icon={<RotateCcw className="h-3.5 w-3.5" />}
          title="Thu hồi tin nhắn"
          onClick={() => void ipc.recallMessage(thread.id, thread.is_group, msg)}
        />
      ) : null}

      <ChatBubbleAction
        icon={<Trash2 className="h-3.5 w-3.5 text-destructive" />}
        title="Xóa chỉ ở phía tôi"
        onClick={() => {
          deleteMessageLocal(thread.id, msg.id, msg.ts);
          setStatusMessage("Đã xóa tin nhắn khỏi lịch sử máy này");
          void ipc.deleteForMe(thread.id, thread.is_group, msg);
        }}
      />
    </ChatBubbleActionWrapper>
  );
}

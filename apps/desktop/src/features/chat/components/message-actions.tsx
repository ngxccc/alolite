import { useState } from "react";
import { CornerUpLeft, Forward, RotateCcw, SmilePlus, Star, Trash2, Undo2 } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import type { ChatMessage, Thread } from "@/shared/types/ipc.generated";
import { Button } from "@/shared/ui/button";
import {
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
} from "@/shared/ui/context-menu";
import { cn } from "cn";
import { useChatStore, useUIStore } from "@/stores";
import {
  CATEGORY_ICONS,
  EMOJI_CATEGORIES,
  PINNED_REACTIONS,
  REACTION_LIST,
} from "../constants/emojis";
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
  const [showAllEmojis, setShowAllEmojis] = useState(false);
  const [category, setCategory] = useState<string>("Smileys");

  const categoryEmojis = EMOJI_CATEGORIES[category] ?? [];

  return (
    <ContextMenuContent className={showAllEmojis ? "w-75" : undefined}>
      {!showAllEmojis ? (
        <div className="flex items-center gap-1 px-1.5 py-1">
          {REACTION_LIST.map((r) => (
            <Button
              key={r.key}
              type="button"
              variant="ghost"
              size="icon-sm"
              title={r.label}
              onClick={() => onSelectReaction(r.emoji)}
              className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-md transition-transform hover:scale-120 hover:bg-accent"
            >
              <ReactionBadge raw={r.emoji} />
            </Button>
          ))}
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            title="Thêm biểu tượng cảm xúc"
            onClick={(e) => {
              e.preventDefault();
              e.stopPropagation();
              setShowAllEmojis(true);
            }}
            className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          >
            <SmilePlus className="h-4 w-4" />
          </Button>
        </div>
      ) : (
        <div className="space-y-2 p-2">
          <div className="flex items-center justify-between border-b border-border pb-1 text-xs text-muted-foreground">
            <span className="font-medium text-foreground">Ghim & Phổ biến</span>
            <Button
              type="button"
              variant="ghost"
              size="icon-xs"
              title="Thu gọn"
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setShowAllEmojis(false);
              }}
              className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              <Undo2 className="h-3.5 w-3.5" />
            </Button>
          </div>
          <div className="flex flex-wrap gap-1">
            {PINNED_REACTIONS.map((em) => (
              <Button
                key={`pin_${em}`}
                type="button"
                variant="ghost"
                size="icon-sm"
                onClick={() => onSelectReaction(em)}
                className="flex h-7 w-7 cursor-pointer items-center justify-center rounded text-base transition-transform hover:scale-120 hover:bg-accent"
              >
                {em}
              </Button>
            ))}
          </div>
          <div className="no-scrollbar flex items-center gap-0.5 overflow-x-auto border-t border-border pt-1">
            {Object.keys(EMOJI_CATEGORIES).map((catName) => (
              <Button
                key={catName}
                type="button"
                variant="ghost"
                size="icon-xs"
                title={catName}
                onClick={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  setCategory(catName);
                }}
                className={cn(
                  "flex h-6 w-6 shrink-0 cursor-pointer items-center justify-center rounded text-sm transition-colors",
                  category === catName
                    ? "bg-accent font-bold text-accent-foreground"
                    : "text-muted-foreground hover:bg-muted",
                )}
              >
                {CATEGORY_ICONS[catName] ?? "😀"}
              </Button>
            ))}
          </div>
          <div className="grid max-h-44 grid-cols-8 gap-1 overflow-y-auto pr-1">
            {categoryEmojis.map((em, idx) => (
              <Button
                key={`${category}_${em}_${idx}`}
                type="button"
                variant="ghost"
                size="icon-sm"
                onClick={() => onSelectReaction(em)}
                className="flex h-7 w-7 cursor-pointer items-center justify-center rounded text-base transition-transform hover:scale-120 hover:bg-accent"
              >
                {em}
              </Button>
            ))}
          </div>
        </div>
      )}
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

import { Pin } from "lucide-react";
import type { ChatMessage } from "@/shared/types/ipc.generated";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

interface PinnedBarProps {
  pinnedMessages: ChatMessage[];
  onJumpToPinned: (msg: ChatMessage) => void;
  onToggleInfoDrawer: () => void;
}

export function PinnedBar({ pinnedMessages, onJumpToPinned }: PinnedBarProps) {
  const pinnedMessage = pinnedMessages.at(0);
  if (!pinnedMessage) return null;

  return (
    <div className="flex shrink-0 items-center justify-between gap-3 border-b border-border bg-muted/40 px-5 py-2">
      <div
        onClick={() => onJumpToPinned(pinnedMessage)}
        className="flex min-w-0 flex-1 cursor-pointer items-center gap-2 overflow-hidden hover:opacity-80"
      >
        <Pin className="h-4 w-4 shrink-0 text-foreground" />
        <span className="shrink-0 text-xs font-medium text-foreground">Ghim:</span>
        <span className="truncate text-xs text-muted-foreground">
          {pinnedMessage.sender ? `${pinnedMessage.sender}: ` : ""}
          {pinnedMessage.text || "[Hình ảnh/Tập tin]"}
        </span>
      </div>
      {pinnedMessages.length > 1 ? (
        <Popover>
          <PopoverTrigger
            render={
              <Button
                type="button"
                variant="link"
                className="h-auto shrink-0 cursor-pointer p-0 text-xs font-medium text-foreground tabular-nums"
              />
            }
          >
            Xem ({pinnedMessages.length})
          </PopoverTrigger>
          <PopoverContent align="end" className="w-85 p-0">
            <div className="border-b border-border px-3 py-2 text-xs font-semibold text-foreground">
              Tin nhắn đã ghim ({pinnedMessages.length})
            </div>
            <div className="flex max-h-64 flex-col divide-y divide-border overflow-y-auto">
              {pinnedMessages.map((pm, idx) => (
                <Button
                  key={pm.id || pm.client_message_id || `${pm.ts}_${idx}`}
                  type="button"
                  variant="ghost"
                  onClick={() => onJumpToPinned(pm)}
                  className="flex h-auto w-full cursor-pointer flex-col items-stretch gap-0.5 rounded-none px-3 py-2 text-left transition-colors hover:bg-accent"
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="truncate text-xs font-medium text-foreground">
                      {pm.sender || "Tin nhắn ghim"}
                    </span>
                    {pm.time ? (
                      <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">
                        {pm.time}
                      </span>
                    ) : null}
                  </div>
                  <span className="line-clamp-2 text-xs text-muted-foreground">
                    {pm.text || "[Hình ảnh/Tập tin]"}
                  </span>
                </Button>
              ))}
            </div>
          </PopoverContent>
        </Popover>
      ) : (
        <Button
          type="button"
          variant="link"
          onClick={() => onJumpToPinned(pinnedMessage)}
          className="h-auto shrink-0 cursor-pointer p-0 text-xs font-medium text-foreground"
        >
          Xem
        </Button>
      )}
    </div>
  );
}

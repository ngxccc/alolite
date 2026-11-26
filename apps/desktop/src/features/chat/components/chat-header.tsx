import { PanelRight, Star, Users } from "lucide-react";
import type { Thread } from "@/shared/types/ipc.generated";
import { UserAvatar } from "@/shared/ui/user-avatar";
import { Button } from "@/shared/ui/button";
import { cn } from "cn";
import { useChatStore, useContactsStore, useMediaCacheStore, useUIStore } from "@/stores";
import { InChatSearch } from "./in-chat-search";

interface ChatHeaderProps {
  thread: Thread;
}

export function ChatHeader({ thread }: ChatHeaderProps) {
  const friendStatus = useContactsStore((s) => s.friendStatusByUid[thread.id]);
  const onlineInfo = useContactsStore((s) => s.userOnlineMap[thread.id]);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);
  const inChatSearchOpen = useUIStore((s) => s.inChatSearchOpen);
  const inChatSearchQuery = useUIStore((s) => s.inChatSearchQuery);
  const setInChatSearchOpen = useUIStore((s) => s.setInChatSearchOpen);
  const setInChatSearchQuery = useUIStore((s) => s.setInChatSearchQuery);
  const filterStarredOnly = useChatStore((s) => s.filterStarredOnly);
  const toggleFilterStarredOnly = useChatStore((s) => s.toggleFilterStarredOnly);
  const infoDrawerOpen = useChatStore((s) => s.infoDrawerOpen);
  const toggleInfoDrawer = useChatStore((s) => s.toggleInfoDrawer);

  return (
    <header className="flex h-16 shrink-0 items-center justify-between border-b border-border bg-background px-5">
      <div className="flex min-w-0 items-center gap-3">
        <UserAvatar
          id={thread.id}
          name={thread.name}
          avatarUrl={thread.avatar}
          isGroup={thread.is_group}
          size={38}
          mediaCache={mediaCache}
        />
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <h1 className="truncate text-sm font-semibold text-foreground">
              {thread.name || thread.id}
            </h1>
            {thread.is_group && thread.presentation.member_count ? (
              <span className="shrink-0 rounded-full border border-border bg-secondary px-2 py-0.5 text-[11px] font-medium text-muted-foreground tabular-nums">
                {thread.presentation.member_count} thành viên
              </span>
            ) : null}
          </div>
          <div
            className={cn(
              "flex items-center gap-1 text-xs",
              onlineInfo?.[0] ? "font-medium text-online" : "text-muted-foreground",
            )}
          >
            {thread.is_group ? (
              <>
                <Users className="h-3 w-3 shrink-0" />
                <span className="truncate">Nhóm trò chuyện</span>
              </>
            ) : thread.name.includes("Cloud của tôi") ? (
              "Lưu trữ đám mây cá nhân"
            ) : (
              onlineInfo?.[1] || (friendStatus === "Stranger" ? "Người lạ" : "Trực tuyến")
            )}
          </div>
        </div>
      </div>

      {/* Right-side icon buttons — 32px each */}
      <div className="flex shrink-0 items-center gap-1">
        <InChatSearch
          isOpen={inChatSearchOpen}
          query={inChatSearchQuery}
          onOpen={() => setInChatSearchOpen(true)}
          onClose={() => setInChatSearchOpen(false)}
          onQueryChange={setInChatSearchQuery}
        />

        <Button
          variant={filterStarredOnly ? "accent" : "ghost"}
          size="icon"
          title="Lọc tin nhắn đã gắn sao"
          className="h-8 w-8 text-muted-foreground hover:text-foreground"
          onClick={toggleFilterStarredOnly}
        >
          <Star className={cn("h-4.5 w-4.5", filterStarredOnly && "text-warning")} />
        </Button>

        {/* Info panel toggle */}
        <Button
          type="button"
          variant={infoDrawerOpen ? "secondary" : "ghost"}
          size="icon"
          title="Thông tin hội thoại"
          aria-pressed={infoDrawerOpen}
          className={cn(
            "h-8 w-8",
            infoDrawerOpen
              ? "border border-border text-foreground"
              : "text-muted-foreground hover:text-foreground",
          )}
          onClick={toggleInfoDrawer}
        >
          <PanelRight className="h-4.5 w-4.5" />
        </Button>
      </div>
    </header>
  );
}

import { PanelRight, Star, Tag, UserPlus, Users } from "lucide-react";
import type { Thread } from "@/shared/types/ipc.generated";
import { Avatar } from "@/shared/ui/avatar";
import { Button } from "@/shared/ui/button";
import { cn } from "@/shared/utils/cn";
import { threadMemberCount } from "@/features/threads/utils/threads";
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
  const setPhoneLookupOpen = useUIStore((s) => s.setPhoneLookupOpen);
  const filterStarredOnly = useChatStore((s) => s.filterStarredOnly);
  const toggleFilterStarredOnly = useChatStore((s) => s.toggleFilterStarredOnly);
  const infoDrawerOpen = useChatStore((s) => s.infoDrawerOpen);
  const toggleInfoDrawer = useChatStore((s) => s.toggleInfoDrawer);

  return (
    <header className="flex h-[68px] shrink-0 items-center justify-between border-b border-border bg-sidebar px-4">
      <div className="flex min-w-0 items-center gap-3">
        <Avatar
          id={thread.id}
          name={thread.name}
          avatarUrl={thread.avatar}
          isGroup={thread.is_group}
          memberCount={thread.is_group ? threadMemberCount(thread) : undefined}
          size={48}
          mediaCache={mediaCache}
        />
        <div className="min-w-0">
          <div className="truncate text-lg font-semibold text-foreground">
            {thread.name || thread.id}
          </div>
          <div
            className={cn(
              "flex items-center gap-1 text-sm",
              onlineInfo?.[0] ? "font-medium text-online" : "text-muted-foreground",
            )}
          >
            {thread.is_group ? (
              <>
                <Users className="h-3.5 w-3.5 shrink-0" />
                <span className="truncate">
                  {thread.presentation.member_count
                    ? `${thread.presentation.member_count} thành viên`
                    : "Nhóm trò chuyện"}
                </span>
                <Tag className="ml-0.5 h-3.5 w-3.5 shrink-0 opacity-60" />
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
      <div className="flex items-center gap-1">
        {thread.is_group ? (
          <Button
            variant="ghost"
            size="icon"
            title="Thêm thành viên vào nhóm"
            className="h-8 w-8 text-muted-foreground hover:text-foreground"
            onClick={() => setPhoneLookupOpen(true)}
          >
            <UserPlus className="h-5 w-5" />
          </Button>
        ) : null}

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
          <Star className={cn("h-5 w-5", filterStarredOnly && "text-warning")} />
        </Button>

        {/* Info panel toggle — filled blue square when active */}
        <button
          type="button"
          title="Thông tin hội thoại"
          className={cn(
            "flex h-8 w-8 cursor-pointer items-center justify-center rounded-md transition-colors",
            infoDrawerOpen
              ? "bg-primary text-primary-foreground"
              : "text-muted-foreground hover:bg-[rgba(255,255,255,0.05)] hover:text-foreground",
          )}
          onClick={toggleInfoDrawer}
        >
          <PanelRight className="h-5 w-5" />
        </button>
      </div>
    </header>
  );
}

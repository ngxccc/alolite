import { useEffect, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  BellOff,
  CheckCheck,
  EyeOff,
  MailCheck,
  MailOpen,
  MoreHorizontal,
  Pin,
  PinOff,
  Search,
  SlidersHorizontal,
  SquarePen,
  Trash2,
  X,
} from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import type { Thread } from "@/shared/types/ipc.generated";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { UserAvatar } from "@/shared/ui/user-avatar";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/shared/ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { cn } from "cn";
import { matchesVnQuery } from "@/shared/utils/text";
import { useChatStore, useContactsStore, useMediaCacheStore, useUIStore } from "@/stores";
import {
  isThreadMuted,
  threadMemberCount,
  threadUnreadCount,
} from "@/features/threads/utils/threads";

export function ThreadsSidebar() {
  const threads = useChatStore((s) => s.threads);
  const selectedThreadId = useChatStore((s) => s.selectedThreadId);
  const threadFilter = useChatStore((s) => s.threadFilter);
  const searchQuery = useChatStore((s) => s.searchQuery);
  const unreadCounts = useChatStore((s) => s.unreadCounts);
  const settings = useChatStore((s) => s.settings);
  const selectThread = useChatStore((s) => s.selectThread);
  const setThreadFilter = useChatStore((s) => s.setThreadFilter);
  const setSearchQuery = useChatStore((s) => s.setSearchQuery);
  const setSettings = useChatStore((s) => s.setSettings);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);
  const setPhoneLookupOpen = useUIStore((s) => s.setPhoneLookupOpen);
  const setActiveSection = useUIStore((s) => s.setActiveSection);
  const setInChatSearchOpen = useUIStore((s) => s.setInChatSearchOpen);
  const setPhoneLookupLoading = useContactsStore((s) => s.setPhoneLookupLoading);
  const setPhoneLookupError = useContactsStore((s) => s.setPhoneLookupError);
  const userOnlineMap = useContactsStore((s) => s.userOnlineMap);

  const listRef = useRef<HTMLDivElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const [unreadOnly, setUnreadOnly] = useState(false);

  const isMac =
    typeof navigator !== "undefined" && /Mac|iPod|iPhone|iPad/i.test(navigator.platform);
  const shortcutLabel = isMac ? "⌘K" : "Ctrl+K";

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  const totalUnread = threads.reduce((sum, t) => sum + threadUnreadCount(t, unreadCounts), 0);

  const hiddenSet = new Set(settings?.hidden_threads ?? []);
  const pinnedSet = new Set(settings?.pinned_threads ?? []);

  const visibleThreads = threads.filter((t) => {
    if (hiddenSet.has(t.id) || t.presentation.hidden) return false;
    if (threadFilter === "direct" && t.is_group) return false;
    if (threadFilter === "groups" && !t.is_group) return false;
    if (unreadOnly) {
      const unread = threadUnreadCount(t, unreadCounts);
      if (unread === 0 && !t.unread.marked_unread) return false;
    }
    if (!searchQuery.trim()) return true;
    return (
      matchesVnQuery(t.name, searchQuery) ||
      matchesVnQuery(t.preview, searchQuery) ||
      t.messages.some((m) => matchesVnQuery(m.text, searchQuery))
    );
  });

  const virtualizer = useVirtualizer({
    count: visibleThreads.length,
    getScrollElement: () => listRef.current,
    estimateSize: () => 58,
    overscan: 8,
  });
  const handleSearchKeyDown = (key: string) => {
    if (key !== "Enter") return;
    const digits = searchQuery.replace(/\D/g, "");
    if (digits.length >= 8) {
      setPhoneLookupOpen(true);
      setPhoneLookupError(null);
      setPhoneLookupLoading(true);
      void ipc.lookupPhone(searchQuery.trim());
    }
  };

  const togglePin = (thread: Thread) => {
    if (!settings) return;
    const isPinned = thread.presentation.pinned || pinnedSet.has(thread.id);
    const nextPinned = isPinned
      ? settings.pinned_threads.filter((id) => id !== thread.id)
      : [...settings.pinned_threads, thread.id];
    const nextSettings = { ...settings, pinned_threads: nextPinned };
    setSettings(nextSettings);
    void ipc.saveSettings(nextSettings);
    void ipc.pinThread(thread.id, thread.is_group, !isPinned);
  };

  const toggleMute = (thread: Thread, durationSec: number, mute: boolean) => {
    if (!settings) return;
    const nextMuted = { ...settings.muted_threads };
    if (mute) {
      nextMuted[thread.id] = durationSec > 0 ? Math.floor(Date.now() / 1000) + durationSec : -1;
    } else {
      delete nextMuted[thread.id];
    }
    const nextSettings = { ...settings, muted_threads: nextMuted };
    setSettings(nextSettings);
    void ipc.saveSettings(nextSettings);
    void ipc.muteThread(thread.id, thread.is_group, durationSec, mute);
  };

  const toggleHide = (thread: Thread) => {
    if (!settings) return;
    const nextHidden = [...settings.hidden_threads, thread.id];
    const nextSettings = { ...settings, hidden_threads: nextHidden };
    setSettings(nextSettings);
    void ipc.saveSettings(nextSettings);
    void ipc.hideThread(thread.id, thread.is_group, true);
  };

  return (
    <section
      aria-label="Danh sách hội thoại"
      className="flex h-full w-75 shrink-0 flex-col border-r border-border bg-sidebar text-sidebar-foreground"
    >
      <header className="flex shrink-0 flex-col gap-3 border-b border-border p-3">
        <div className="flex items-center justify-between gap-2">
          <h2 className="text-sm font-semibold tracking-tight text-foreground">Tin nhắn</h2>
          <div className="flex items-center gap-1">
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    type="button"
                    variant={threadFilter !== "all" ? "secondary" : "ghost"}
                    size="icon"
                    title={
                      threadFilter === "direct"
                        ? "Đang lọc: Cá nhân"
                        : threadFilter === "groups"
                          ? "Đang lọc: Nhóm"
                          : "Phân loại hội thoại"
                    }
                    className={cn(
                      "inline-flex h-8 w-8 cursor-pointer items-center justify-center rounded-md transition-colors",
                      threadFilter !== "all"
                        ? "border border-border text-foreground"
                        : "text-muted-foreground hover:text-foreground",
                    )}
                  />
                }
              >
                <SlidersHorizontal className="h-4 w-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onClick={() => setThreadFilter("all")}>
                  Tất cả phân loại
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => setThreadFilter("direct")}>
                  Tin nhắn cá nhân
                </DropdownMenuItem>
                <DropdownMenuItem onClick={() => setThreadFilter("groups")}>
                  Tin nhắn nhóm
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>

            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    title="Tùy chọn khác"
                    className="inline-flex h-8 w-8 cursor-pointer items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  />
                }
              >
                <MoreHorizontal className="h-4 w-4" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem
                  onClick={() => {
                    for (const t of threads) {
                      const unread = threadUnreadCount(t, unreadCounts);
                      if (unread > 0 || t.unread.marked_unread) {
                        useChatStore.getState().markThreadRead(t.id);
                        const newestNumeric =
                          t.messages.toReversed().find((m) => /^\d+$/.test(m.id))?.id ??
                          t.messages.at(-1)?.id ??
                          "";
                        void ipc.sendSeen(t.id, t.is_group, newestNumeric);
                      }
                    }
                  }}
                >
                  <CheckCheck className="h-4 w-4" /> Đánh dấu đã đọc tất cả
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>

            <Button
              type="button"
              variant="outline"
              aria-label="Tạo hội thoại hoặc tìm số điện thoại"
              onClick={() => setPhoneLookupOpen(true)}
              className="inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-md border border-border bg-background px-2.5 text-xs font-medium text-foreground transition-colors hover:bg-accent"
            >
              <SquarePen className="h-3.5 w-3.5" />
              <span>Mới</span>
            </Button>
          </div>
        </div>

        {/* Command Search Input */}
        <div
          onClick={() => searchInputRef.current?.focus()}
          className="flex h-9 cursor-text items-center gap-2 rounded-md border border-input bg-background px-3 focus-within:ring-1 focus-within:ring-ring"
        >
          <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
          <Input
            ref={searchInputRef}
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setSearchQuery("");
                searchInputRef.current?.blur();
                return;
              }
              handleSearchKeyDown(e.key);
            }}
            placeholder="Tìm hội thoại..."
            className="h-auto w-full border-0 bg-transparent p-0 text-sm text-foreground shadow-none placeholder:text-muted-foreground focus-visible:ring-0 dark:bg-transparent"
          />
          {searchQuery ? (
            <Button
              type="button"
              variant="ghost"
              size="icon-xs"
              onClick={() => setSearchQuery("")}
              title="Xóa tìm kiếm"
              className="inline-flex h-4 w-4 shrink-0 cursor-pointer items-center justify-center p-0 text-muted-foreground hover:text-foreground"
            >
              <X className="h-3 w-3" />
            </Button>
          ) : (
            <kbd className="shrink-0 rounded border border-border bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
              {shortcutLabel}
            </kbd>
          )}
        </div>

        {/* Segmented TabsList */}
        <div className="grid grid-cols-2 rounded-md bg-muted p-1 text-muted-foreground">
          <Button
            type="button"
            variant="ghost"
            onClick={() => setUnreadOnly(false)}
            className={cn(
              "h-auto cursor-pointer rounded-sm py-1 text-xs font-medium transition-colors",
              !unreadOnly
                ? "bg-background text-foreground shadow-sm hover:bg-background"
                : "text-muted-foreground hover:bg-transparent hover:text-foreground",
            )}
          >
            Tất cả
          </Button>
          <Button
            type="button"
            variant="ghost"
            onClick={() => setUnreadOnly(true)}
            className={cn(
              "h-auto cursor-pointer rounded-sm py-1 text-xs font-medium tabular-nums transition-colors",
              unreadOnly
                ? "bg-background text-foreground shadow-sm hover:bg-background"
                : "text-muted-foreground hover:bg-transparent hover:text-foreground",
            )}
          >
            {totalUnread > 0 ? `Chưa đọc (${totalUnread > 99 ? "99+" : totalUnread})` : "Chưa đọc"}
          </Button>
        </div>
      </header>

      {/* Conversation list — virtualised, items use inset-x-2 for 8 px side margins */}
      <div ref={listRef} className="flex-1 overflow-y-auto py-1">
        {visibleThreads.length === 0 ? (
          <div className="p-6 text-center text-sm text-muted-foreground">
            Không có hội thoại phù hợp
          </div>
        ) : (
          <div style={{ height: virtualizer.getTotalSize() }} className="relative w-full">
            {virtualizer.getVirtualItems().map((vRow) => {
              const t = visibleThreads.at(vRow.index);
              if (!t) return null;
              const selected = t.id === selectedThreadId;
              const muted = settings ? isThreadMuted(t, settings) : false;
              const pinned = t.presentation.pinned || pinnedSet.has(t.id);
              const unread = threadUnreadCount(t, unreadCounts);
              const hasDot = unread > 0 || t.unread.marked_unread;
              const isOnline = !t.is_group && Boolean(userOnlineMap[t.id]?.[0]);

              return (
                <ContextMenu key={t.id}>
                  <ContextMenuTrigger
                    render={
                      <div
                        style={{
                          height: vRow.size,
                          transform: `translateY(${vRow.start}px)`,
                        }}
                        onClick={() => {
                          setActiveSection("chat");
                          setInChatSearchOpen(false);
                          const hadUnread =
                            threadUnreadCount(t, unreadCounts) > 0 || t.unread.marked_unread;
                          selectThread(t.id);
                          const last = t.messages.at(-1);
                          const newestNumeric =
                            t.messages.toReversed().find((m) => /^\d+$/.test(m.id))?.id ??
                            last?.id ??
                            "";
                          if (hadUnread || (last && !last.is_outgoing && newestNumeric)) {
                            void ipc.sendSeen(t.id, t.is_group, newestNumeric);
                          }
                        }}
                        className={cn(
                          "absolute inset-x-2 top-0 flex cursor-pointer items-center gap-3 rounded-md px-2.5 transition-colors select-none",
                          selected ? "bg-muted text-foreground" : "hover:bg-muted/60",
                        )}
                      />
                    }
                  >
                    <div className="relative shrink-0">
                      <UserAvatar
                        id={t.id}
                        name={t.name}
                        avatarUrl={t.avatar}
                        isGroup={t.is_group}
                        memberCount={t.is_group ? threadMemberCount(t) : undefined}
                        size={36}
                        mediaCache={mediaCache}
                      />
                      {isOnline ? (
                        <span className="absolute right-0 bottom-0 h-2.5 w-2.5 rounded-full bg-online ring-2 ring-sidebar" />
                      ) : null}
                    </div>
                    <div className="flex min-w-0 flex-1 flex-col">
                      <div className="flex items-center justify-between gap-2">
                        <span
                          className={cn(
                            "truncate text-sm text-foreground",
                            selected || hasDot ? "font-semibold" : "font-medium",
                          )}
                        >
                          {t.name || t.id}
                        </span>
                        <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">
                          {t.time}
                        </span>
                      </div>
                      <div className="flex items-center justify-between gap-2">
                        <span
                          className={cn(
                            "truncate text-xs",
                            hasDot ? "font-medium text-foreground" : "text-muted-foreground",
                          )}
                        >
                          {t.presentation.last_outgoing &&
                          !t.preview.startsWith("Bạn:") &&
                          !t.preview.startsWith("You:")
                            ? `You: ${t.preview || "Hình ảnh"}`
                            : t.is_group && t.presentation.last_sender && !t.preview.includes(":")
                              ? `${t.presentation.last_sender}: ${t.preview}`
                              : t.preview || "Chưa có tin nhắn"}
                        </span>
                        <div className="inline-flex shrink-0 items-center gap-1">
                          {pinned ? <Pin className="h-3 w-3 text-muted-foreground" /> : null}
                          {muted ? <BellOff className="h-3 w-3 text-muted-foreground" /> : null}
                          {unread > 0 ? (
                            <span className="inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-[10px] font-semibold text-primary-foreground tabular-nums">
                              {unread > 99 ? "99+" : unread}
                            </span>
                          ) : t.unread.marked_unread ? (
                            <span className="h-2 w-2 rounded-full bg-primary" />
                          ) : null}
                        </div>
                      </div>
                    </div>
                  </ContextMenuTrigger>

                  <ContextMenuContent>
                    <ContextMenuItem onClick={() => togglePin(t)}>
                      {pinned ? (
                        <>
                          <PinOff className="h-4 w-4" /> Bỏ ghim hội thoại
                        </>
                      ) : (
                        <>
                          <Pin className="h-4 w-4" /> Ghim hội thoại
                        </>
                      )}
                    </ContextMenuItem>
                    <ContextMenuItem
                      onClick={() => {
                        const markUnread = !t.unread.marked_unread;
                        void ipc.markThreadUnread(t.id, t.is_group, markUnread);
                      }}
                    >
                      {t.unread.marked_unread ? (
                        <>
                          <MailOpen className="h-4 w-4" /> Đánh dấu đã đọc
                        </>
                      ) : (
                        <>
                          <MailCheck className="h-4 w-4" /> Đánh dấu chưa đọc
                        </>
                      )}
                    </ContextMenuItem>
                    {muted ? (
                      <ContextMenuItem onClick={() => toggleMute(t, 0, false)}>
                        <BellOff className="h-4 w-4" /> Bật lại thông báo
                      </ContextMenuItem>
                    ) : (
                      <>
                        <ContextMenuItem onClick={() => toggleMute(t, 3600, true)}>
                          <BellOff className="h-4 w-4" /> Tắt thông báo 1 giờ
                        </ContextMenuItem>
                        <ContextMenuItem onClick={() => toggleMute(t, -1, true)}>
                          <BellOff className="h-4 w-4" /> Tắt đến khi bật lại
                        </ContextMenuItem>
                      </>
                    )}
                    <ContextMenuItem onClick={() => toggleHide(t)}>
                      <EyeOff className="h-4 w-4" /> Ẩn trò chuyện
                    </ContextMenuItem>
                    <ContextMenuSeparator />
                    <ContextMenuItem
                      variant="destructive"
                      onClick={() => {
                        const last = t.messages.at(-1);
                        void ipc.deleteThread(
                          t.id,
                          t.is_group,
                          last?.client_message_id ?? last?.id ?? "",
                        );
                      }}
                    >
                      <Trash2 className="h-4 w-4" /> Xóa hội thoại
                    </ContextMenuItem>
                  </ContextMenuContent>
                </ContextMenu>
              );
            })}
          </div>
        )}
      </div>
    </section>
  );
}

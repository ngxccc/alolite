import { useState } from "react";
import { Search, UserPlus, Users, UserCheck, ArrowUpDown } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Input } from "@/shared/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { UserAvatar } from "@/shared/ui/user-avatar";
import { cn } from "cn";
import { foldVnSearch, matchesVnQuery } from "@/shared/utils/text";
import type { Thread } from "@/shared/types/ipc.generated";
import {
  useChatStore,
  useContactsStore,
  useMediaCacheStore,
  useUIStore,
  type ContactsSubTab,
} from "@/stores";

// Nav items in the left column
const CONTACT_TABS: {
  id: ContactsSubTab;
  label: string;
  icon: React.ComponentType<{ className?: string }>;
}[] = [
  { id: "friends", label: "Danh sách bạn bè", icon: UserCheck },
  { id: "groups", label: "Danh sách nhóm và cộng đồng", icon: Users },
  { id: "friend_requests", label: "Lời mời kết bạn", icon: UserPlus },
];

// Header display label for each sub-tab
const TAB_HEADER: Record<ContactsSubTab, string> = {
  friends: "Danh sách bạn bè",
  groups: "Danh sách nhóm và cộng đồng",
  friend_requests: "Lời mời kết bạn",
};

// Caption prefix for each sub-tab
const TAB_CAPTION: Record<ContactsSubTab, string> = {
  friends: "Bạn bè",
  groups: "Nhóm và cộng đồng",
  friend_requests: "Yêu cầu nhận được",
};

type SortKey = "name_asc" | "name_desc" | "recent";

/** Strip Vietnamese diacritics and return uppercase first letter for grouping */
function groupLetter(name: string): string {
  const folded = foldVnSearch(name.trim());
  const firstChar = folded.charAt(0).toUpperCase();
  if (!firstChar) return "#";
  return /[A-Z]/.test(firstChar) ? firstChar : "#";
}

export function ContactsView() {
  const sub = useContactsStore((s) => s.contactsSubTab);
  const contactsSearchQuery = useContactsStore((s) => s.contactsSearchQuery);
  const friendStatusMap = useContactsStore((s) => s.friendStatusByUid);
  const setContactsSubTab = useContactsStore((s) => s.setContactsSubTab);
  const setContactsSearchQuery = useContactsStore((s) => s.setContactsSearchQuery);
  const threads = useChatStore((s) => s.threads);
  const selectThread = useChatStore((s) => s.selectThread);
  const setPhoneLookupOpen = useUIStore((s) => s.setPhoneLookupOpen);
  const setActiveSection = useUIStore((s) => s.setActiveSection);
  const setInChatSearchOpen = useUIStore((s) => s.setInChatSearchOpen);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);

  const openThread = (threadId: string) => {
    setActiveSection("chat");
    setInChatSearchOpen(false);
    selectThread(threadId);
  };

  // Per-tab search query (local state)
  const [localSearch, setLocalSearch] = useState("");
  const [userSortKey, setUserSortKey] = useState<SortKey | null>(null);

  const sortKey: SortKey = userSortKey ?? (sub === "groups" ? "recent" : "name_asc");

  // Build items list based on the active sub-tab
  const rawItems = threads.filter((t) => {
    if (sub === "friend_requests") {
      return !t.is_group && friendStatusMap[t.id] === "IncomingPending";
    }
    if (sub === "groups") {
      return t.is_group && !t.name.includes("Cloud của tôi");
    }
    // friends: non-group, not incoming pending, not Cloud của tôi
    return (
      !t.is_group &&
      !t.name.includes("Cloud của tôi") &&
      friendStatusMap[t.id] !== "IncomingPending"
    );
  });

  // Apply search filter (from local search input or left sidebar search input)
  const searchQuery = (localSearch || contactsSearchQuery).trim();
  const searched = searchQuery
    ? rawItems.filter((t) => matchesVnQuery(t.name, searchQuery))
    : rawItems;
  // Sort
  const sorted = [...searched].sort((a, b) => {
    if (sortKey === "name_asc") return foldVnSearch(a.name).localeCompare(foldVnSearch(b.name));
    if (sortKey === "name_desc") return foldVnSearch(b.name).localeCompare(foldVnSearch(a.name));
    // recent: last_message_ts descending
    return b.presentation.last_message_ts - a.presentation.last_message_ts;
  });

  const headerIcon = (() => {
    const tab = CONTACT_TABS.find((t) => t.id === sub);
    if (!tab) return null;
    const Icon = tab.icon;
    return <Icon className="h-4.5 w-4.5 text-muted-foreground" />;
  })();

  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      {/* Left nav column — 345px */}
      <aside className="flex h-full w-86.25 shrink-0 flex-col border-r border-border bg-background">
        {/* Top row: 64px, search + add-friend */}
        <div className="flex h-16 shrink-0 items-center gap-1.5 px-3">
          {/* Search input */}
          <div className="flex h-8 flex-1 items-center gap-1.5 rounded-[5px] bg-canvas px-2.5">
            <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <Input
              value={contactsSearchQuery}
              onChange={(e) => setContactsSearchQuery(e.target.value)}
              placeholder="Tìm kiếm"
              className="h-auto min-w-0 flex-1 border-0 bg-transparent p-0 text-sm text-foreground shadow-none placeholder:text-muted-foreground focus-visible:ring-0 dark:bg-transparent"
            />
          </div>
          {/* Add friend button (opens PhoneLookupModal) */}
          <Button
            type="button"
            variant="ghost"
            size="icon"
            title="Thêm bạn"
            onClick={() => setPhoneLookupOpen(true)}
            className="flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-foreground/5 hover:text-foreground"
          >
            <UserPlus className="h-5 w-5" />
          </Button>
        </div>

        {/* Nav items */}
        <nav className="flex flex-col gap-0.5 px-2 py-1">
          {CONTACT_TABS.map((tab) => {
            const Icon = tab.icon;
            const active = sub === tab.id;
            return (
              <Button
                key={tab.id}
                type="button"
                variant="ghost"
                onClick={() => {
                  setContactsSubTab(tab.id);
                  setLocalSearch("");
                  setUserSortKey(null);
                }}
                className={cn(
                  "flex h-12 w-full cursor-pointer items-center justify-start gap-3 rounded-[6px] px-3 text-left text-sm font-normal transition-colors",
                  active ? "bg-accent text-foreground" : "text-foreground hover:bg-foreground/5",
                )}
              >
                <Icon
                  className={cn(
                    "h-5 w-5 shrink-0",
                    active ? "text-foreground" : "text-muted-foreground",
                  )}
                />
                <span className="truncate">{tab.label}</span>
              </Button>
            );
          })}
        </nav>
      </aside>

      {/* Right main area */}
      <main className="flex min-w-0 flex-1 flex-col bg-canvas">
        {/* Header: 52px */}
        <header className="flex h-13 shrink-0 items-center gap-2.5 border-b border-border bg-background px-4">
          {headerIcon}
          <span className="text-base font-medium text-foreground">{TAB_HEADER[sub]}</span>
        </header>

        {/* Body (scrollable) */}
        <div className="flex-1 overflow-y-auto px-4 py-3">
          {sub === "friend_requests" ? (
            <FriendRequestsPanel items={sorted} mediaCache={mediaCache} />
          ) : (
            <ContactListPanel
              sub={sub}
              items={sorted}
              mediaCache={mediaCache}
              localSearch={localSearch}
              onSearch={setLocalSearch}
              sortKey={sortKey}
              onSort={setUserSortKey}
              onSelectThread={openThread}
            />
          )}
        </div>
      </main>
    </div>
  );
}

// ─── Friends / Groups list panel ─────────────────────────────────────────────

interface ContactListPanelProps {
  sub: ContactsSubTab;
  items: Thread[];
  mediaCache: Record<string, string | undefined>;
  localSearch: string;
  onSearch: (q: string) => void;
  sortKey: SortKey;
  onSort: (k: SortKey) => void;
  onSelectThread: (threadId: string) => void;
}

function ContactListPanel({
  sub,
  items,
  mediaCache,
  localSearch,
  onSearch,
  sortKey,
  onSort,
  onSelectThread,
}: ContactListPanelProps) {
  const isGroups = sub === "groups";
  const captionLabel = TAB_CAPTION[sub];

  // Build grouped sections (friends only; groups are flat)
  const sections: { letter: string; rows: Thread[] }[] = [];
  if (!isGroups) {
    const map = new Map<string, Thread[]>();
    for (const item of items) {
      const letter = groupLetter(item.name);
      let list = map.get(letter);
      if (!list) {
        list = [];
        map.set(letter, list);
      }
      list.push(item);
    }
    for (const [letter, rows] of [...map.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
      sections.push({ letter, rows });
    }
  }

  return (
    <>
      {/* Caption */}
      <div className="mb-3 text-sm font-medium text-foreground">
        {captionLabel} ({items.length})
      </div>

      {/* Card */}
      <div className="rounded-lg bg-card">
        {/* Toolbar row */}
        <div className="flex items-center gap-2 px-3 py-2.5">
          {/* Search */}
          <div className="flex h-8 flex-1 items-center gap-1.5 rounded-[5px] bg-canvas px-2.5">
            <Search className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <Input
              value={localSearch}
              onChange={(e) => onSearch(e.target.value)}
              placeholder={isGroups ? "Tìm kiếm..." : "Tìm bạn"}
              className="h-auto min-w-0 flex-1 border-0 bg-transparent p-0 text-sm text-foreground shadow-none placeholder:text-muted-foreground focus-visible:ring-0 dark:bg-transparent"
            />
          </div>
          {/* Sort select */}
          <Select
            value={sortKey}
            items={
              isGroups
                ? [
                    { value: "recent", label: "Cập nhật gần nhất" },
                    { value: "name_asc", label: "Tên (A-Z)" },
                    { value: "name_desc", label: "Tên (Z-A)" },
                  ]
                : [
                    { value: "name_asc", label: "Tên (A-Z)" },
                    { value: "name_desc", label: "Tên (Z-A)" },
                  ]
            }
            onValueChange={(v) => v && onSort(v)}
          >
            <SelectTrigger className="h-8 w-auto min-w-35 gap-1.5 border-border bg-canvas px-2.5 text-sm">
              <ArrowUpDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {isGroups ? (
                <>
                  <SelectItem value="recent">Cập nhật gần nhất</SelectItem>
                  <SelectItem value="name_asc">Tên (A-Z)</SelectItem>
                  <SelectItem value="name_desc">Tên (Z-A)</SelectItem>
                </>
              ) : (
                <>
                  <SelectItem value="name_asc">Tên (A-Z)</SelectItem>
                  <SelectItem value="name_desc">Tên (Z-A)</SelectItem>
                </>
              )}
            </SelectContent>
          </Select>
        </div>

        {/* Rows */}
        {items.length === 0 ? (
          <div className="px-4 py-8 text-center text-sm text-muted-foreground">
            Không có mục nào phù hợp
          </div>
        ) : isGroups ? (
          // Groups: flat list, no letter headings
          items.map((t) => (
            <GroupRow
              key={t.id}
              thread={t}
              mediaCache={mediaCache}
              onSelectThread={onSelectThread}
            />
          ))
        ) : (
          // Friends: alphabetical sections
          sections.map(({ letter, rows }) => (
            <div key={letter}>
              <div className="px-4 pt-3 pb-1 text-base font-medium text-foreground">{letter}</div>
              {rows.map((t) => (
                <FriendRow
                  key={t.id}
                  thread={t}
                  mediaCache={mediaCache}
                  onSelectThread={onSelectThread}
                />
              ))}
            </div>
          ))
        )}
      </div>
    </>
  );
}

// ─── Individual row components ────────────────────────────────────────────────

interface RowProps {
  thread: Thread;
  mediaCache: Record<string, string | undefined>;
  onSelectThread: (threadId: string) => void;
}

function FriendRow({ thread, mediaCache, onSelectThread }: RowProps) {
  return (
    <div
      onClick={() => onSelectThread(thread.id)}
      className="group flex h-14 cursor-pointer items-center gap-3 px-4 hover:bg-foreground/5"
    >
      <UserAvatar
        id={thread.id}
        name={thread.name}
        avatarUrl={thread.avatar}
        isGroup={false}
        size={40}
        mediaCache={mediaCache}
      />
      <span className="min-w-0 flex-1 truncate text-base font-medium text-foreground">
        {thread.name}
      </span>
    </div>
  );
}

function GroupRow({ thread, mediaCache, onSelectThread }: RowProps) {
  const memberCount = thread.presentation.member_count;
  return (
    <div
      onClick={() => onSelectThread(thread.id)}
      className="group flex h-15 cursor-pointer items-center gap-3 px-4 hover:bg-foreground/5"
    >
      <UserAvatar
        id={thread.id}
        name={thread.name}
        avatarUrl={thread.avatar}
        isGroup
        memberCount={memberCount ?? undefined}
        size={40}
        mediaCache={mediaCache}
      />
      <div className="min-w-0 flex-1">
        <div className="truncate text-base font-medium text-foreground">{thread.name}</div>
        {memberCount != null && (
          <div className="text-xs text-muted-foreground">{memberCount} thành viên</div>
        )}
      </div>
    </div>
  );
}
// ─── Friend Requests panel ────────────────────────────────────────────────────

interface FriendRequestsPanelProps {
  items: Thread[];
  mediaCache: Record<string, string | undefined>;
}

function FriendRequestsPanel({ items, mediaCache }: FriendRequestsPanelProps) {
  return (
    <>
      <div className="mb-3 text-sm font-medium text-foreground">
        Yêu cầu nhận được ({items.length})
      </div>
      {items.length === 0 ? (
        <div className="rounded-lg bg-card px-4 py-8 text-center text-sm text-muted-foreground">
          Không có lời mời kết bạn nào
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          {items.map((t) => (
            <FriendRequestCard key={t.id} thread={t} mediaCache={mediaCache} />
          ))}
        </div>
      )}
    </>
  );
}

interface FriendRequestCardProps {
  thread: Thread;
  mediaCache: Record<string, string | undefined>;
}

function FriendRequestCard({ thread, mediaCache }: FriendRequestCardProps) {
  return (
    <Card className="w-99.25 gap-0 p-3">
      {/* Top row: avatar + name + time + icon */}
      <div className="mb-2 flex items-start gap-3">
        <UserAvatar
          id={thread.id}
          name={thread.name}
          avatarUrl={thread.avatar}
          isGroup={false}
          size={40}
          mediaCache={mediaCache}
        />
        <div className="min-w-0 flex-1">
          <div className="text-base font-medium text-foreground">{thread.name}</div>
          <div className="text-sm text-muted-foreground">{thread.time} - Từ số điện thoại</div>
        </div>
      </div>

      {/* Message box (read-only textarea appearance) */}
      <div className="mb-3 min-h-15 rounded-md border border-border bg-canvas px-3 py-2 text-sm text-muted-foreground">
        Xin chào, mình là {thread.name}. Mình biết bạn qua số điện thoại.
      </div>

      {/* Action buttons */}
      <div className="flex gap-2">
        <Button
          variant="secondary"
          className="h-8 flex-1"
          onClick={() => void ipc.rejectFriend(thread.id, thread.name)}
        >
          Từ chối
        </Button>
        <Button
          variant="default"
          className="h-8 flex-1"
          onClick={() => void ipc.acceptFriend(thread.id, thread.name)}
        >
          Đồng ý
        </Button>
      </div>
    </Card>
  );
}

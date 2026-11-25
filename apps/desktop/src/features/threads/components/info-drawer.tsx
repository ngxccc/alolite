import { useState } from "react";
import {
  Bell,
  BellOff,
  ChevronDown,
  ChevronRight,
  Clock,
  FileText,
  Pencil,
  Pin,
  Shield,
  Trash2,
  UserCheck,
  UserPlus,
  Users,
  UserX,
  X,
} from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import { UserAvatar } from "@/shared/ui/user-avatar";
import { Button } from "@/shared/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import { cn } from "cn";
import { jxlToJpgUrl } from "@/shared/utils/text";
import { useChatStore, useContactsStore, useMediaCacheStore, useUIStore } from "@/stores";
import { isFileMessage, isPhotoMessage, isVideoMessage } from "@/features/chat/utils/messages";
import { isThreadMuted, threadMemberCount } from "@/features/threads/utils/threads";

/** Collapsible section header row */
function SectionHeader({
  label,
  expanded,
  onToggle,
  trailing,
}: {
  label: string;
  expanded: boolean;
  onToggle: () => void;
  trailing?: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      className="flex w-full cursor-pointer items-center justify-between px-4 py-2.5 text-left hover:bg-accent/50"
    >
      <span className="text-xs font-semibold text-foreground">{label}</span>
      <div className="flex items-center gap-1.5 text-muted-foreground">
        {trailing}
        {expanded ? <ChevronDown className="h-4 w-4" /> : <ChevronRight className="h-4 w-4" />}
      </div>
    </button>
  );
}

export function InfoDrawer() {
  const threads = useChatStore((s) => s.threads);
  const selectedThreadId = useChatStore((s) => s.selectedThreadId);
  const infoDrawerOpen = useChatStore((s) => s.infoDrawerOpen);
  const settings = useChatStore((s) => s.settings);
  const setSettings = useChatStore((s) => s.setSettings);
  const toggleInfoDrawer = useChatStore((s) => s.toggleInfoDrawer);
  const friendStatusMap = useContactsStore((s) => s.friendStatusByUid);
  const userOnlineMap = useContactsStore((s) => s.userOnlineMap);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);
  const setPhoneLookupOpen = useUIStore((s) => s.setPhoneLookupOpen);
  const setSettingsOpen = useUIStore((s) => s.setSettingsOpen);
  const setImageViewerState = useUIStore((s) => s.setImageViewerState);

  const [membersExpanded, setMembersExpanded] = useState(true);
  const [boardExpanded, setBoardExpanded] = useState(false);

  const thread = threads.find((t) => t.id === selectedThreadId);
  if (!infoDrawerOpen || !thread) return null;

  const isCloud = thread.name.includes("Cloud của tôi");
  const friendStatus = friendStatusMap[thread.id] ?? "Unknown";
  const online = userOnlineMap[thread.id];
  const muted = settings ? isThreadMuted(thread, settings) : false;
  const pinned =
    thread.presentation.pinned || (settings?.pinned_threads.includes(thread.id) ?? false);
  const memberCount = threadMemberCount(thread);

  const sharedPhotos = thread.messages
    .filter((m) => isPhotoMessage(m) && Boolean(m.link_thumb ?? m.link_url))
    .slice(-12)
    .toReversed();

  const sharedFiles = thread.messages
    .filter((m) => isFileMessage(m) && Boolean(m.link_url || m.link_title || m.text))
    .slice(-10)
    .toReversed();

  const sharedLinks = thread.messages
    .filter(
      (m) => Boolean(m.link_url) && !isPhotoMessage(m) && !isFileMessage(m) && !isVideoMessage(m),
    )
    .slice(-10)
    .toReversed();

  const handleTogglePin = () => {
    if (settings) {
      const nextPinned = pinned
        ? settings.pinned_threads.filter((id) => id !== thread.id)
        : [...settings.pinned_threads, thread.id];
      const nextSettings = {
        ...settings,
        pinned_threads: nextPinned,
      };
      setSettings(nextSettings);
      void ipc.saveSettings(nextSettings);
    }
    void ipc.pinThread(thread.id, thread.is_group, !pinned);
  };

  const handleToggleMute = () => {
    if (settings) {
      const nextMuted = { ...settings.muted_threads };
      if (muted) delete nextMuted[thread.id];
      else nextMuted[thread.id] = -1;
      const nextSettings = {
        ...settings,
        muted_threads: nextMuted,
      };
      setSettings(nextSettings);
      void ipc.saveSettings(nextSettings);
    }
    void ipc.muteThread(thread.id, thread.is_group, muted ? 0 : -1, !muted);
  };

  /** Circular action button used in the quick-action row */
  const ActionBtn = ({
    icon,
    label,
    onClick,
    active = false,
  }: {
    icon: React.ReactNode;
    label: string;
    onClick?: () => void;
    active?: boolean;
  }) => (
    <button
      type="button"
      onClick={onClick}
      className="group flex cursor-pointer flex-col items-center gap-1.5"
    >
      <div
        className={cn(
          "flex h-10 w-10 items-center justify-center rounded-full transition-colors",
          active
            ? "bg-primary/20 text-primary"
            : "bg-secondary text-foreground/80 group-hover:bg-secondary/70",
        )}
      >
        {icon}
      </div>
      <span className="text-xs leading-tight text-muted-foreground group-hover:text-foreground">
        {label}
      </span>
    </button>
  );

  /* ------------------------------------------------------------------ */
  /* Action button sets per thread type                                   */
  /* ------------------------------------------------------------------ */
  const actionButtons = thread.is_group ? (
    // Group: 4 buttons
    <div className="grid grid-cols-4 gap-1 px-3 py-4">
      <ActionBtn
        icon={muted ? <BellOff className="h-5 w-5" /> : <Bell className="h-5 w-5" />}
        label={muted ? "Bật TB" : "Tắt thông báo"}
        onClick={handleToggleMute}
        active={muted}
      />
      <ActionBtn
        icon={<Pin className="h-5 w-5" />}
        label={pinned ? "Bỏ ghim" : "Ghim"}
        onClick={handleTogglePin}
        active={pinned}
      />
      <ActionBtn
        icon={<UserPlus className="h-5 w-5" />}
        label="Thêm thành viên"
        onClick={() => setPhoneLookupOpen(true)}
      />
      <ActionBtn
        icon={<Shield className="h-5 w-5" />}
        label="Quản lý nhóm"
        onClick={() => setSettingsOpen(true, "general")}
      />
    </div>
  ) : (
    // 1-on-1 (non-cloud): 3 buttons
    <div className="grid grid-cols-3 gap-1 px-3 py-4">
      <ActionBtn
        icon={muted ? <BellOff className="h-5 w-5" /> : <Bell className="h-5 w-5" />}
        label={muted ? "Bật TB" : "Tắt thông báo"}
        onClick={handleToggleMute}
        active={muted}
      />
      <ActionBtn
        icon={<Pin className="h-5 w-5" />}
        label={pinned ? "Bỏ ghim" : "Ghim"}
        onClick={handleTogglePin}
        active={pinned}
      />
      <ActionBtn
        icon={<Users className="h-5 w-5" />}
        label="Tạo nhóm trò chuyện"
        onClick={() => setPhoneLookupOpen(true)}
      />
    </div>
  );

  return (
    <aside
      aria-label="Thông tin hội thoại"
      className="fixed inset-y-0 right-0 z-40 flex h-full w-[300px] shrink-0 flex-col overflow-y-auto border-l border-border bg-background/95 text-foreground shadow-xl backdrop-blur-xs lg:static lg:bg-muted/20 lg:shadow-none"
    >
      {/* Header: 64px */}
      <div className="relative flex h-16 shrink-0 items-center justify-center border-b border-border px-4">
        <span className="text-sm font-semibold text-foreground">
          {thread.is_group ? "Thông tin nhóm" : "Thông tin hội thoại"}
        </span>
        <button
          type="button"
          aria-label="Đóng thông tin"
          onClick={toggleInfoDrawer}
          className="absolute right-3 inline-flex h-7 w-7 cursor-pointer items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground lg:hidden"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {/* Avatar + Name row */}
      <div className="flex flex-col items-center gap-1.5 border-b border-border px-4 py-4 text-center">
        <UserAvatar
          id={thread.id}
          name={thread.name}
          avatarUrl={thread.avatar}
          isGroup={thread.is_group}
          size={56}
          mediaCache={mediaCache}
        />
        <div className="mt-1 flex items-center gap-1.5">
          <span className="max-w-[220px] truncate text-sm font-semibold text-foreground">
            {thread.name || thread.id}
          </span>
          <button
            type="button"
            title="Chỉnh sửa tên"
            className="inline-flex h-6 w-6 cursor-pointer items-center justify-center rounded text-muted-foreground hover:text-foreground"
          >
            <Pencil className="h-3.5 w-3.5" />
          </button>
        </div>

        {/* Subtitle */}
        <div className="text-xs text-muted-foreground tabular-nums">
          {thread.is_group
            ? `${memberCount} thành viên`
            : isCloud
              ? "Lưu trữ đám mây cá nhân"
              : friendStatus === "Friend"
                ? `Đã kết bạn${online?.[1] ? ` · ${online[1]}` : ""}`
                : friendStatus === "Stranger"
                  ? "Người lạ · Chưa kết bạn"
                  : "Đang kiểm tra…"}
        </div>

        {/* Friend action buttons (non-group, non-cloud, not Friend) */}
        {!thread.is_group && !isCloud && friendStatus !== "Friend" ? (
          <div className="mt-2 flex flex-wrap justify-center gap-1.5">
            {friendStatus === "Stranger" ? (
              <Button
                variant="default"
                size="sm"
                onClick={() => void ipc.addFriend(thread.id, thread.name)}
              >
                <UserPlus className="h-3.5 w-3.5" /> Kết bạn
              </Button>
            ) : null}
            {friendStatus === "IncomingPending" ? (
              <>
                <Button
                  variant="default"
                  size="sm"
                  onClick={() => void ipc.acceptFriend(thread.id, thread.name)}
                >
                  <UserCheck className="h-3.5 w-3.5" /> Đồng ý
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => void ipc.rejectFriend(thread.id, thread.name)}
                >
                  <UserX className="h-3.5 w-3.5" /> Từ chối
                </Button>
              </>
            ) : null}
            {friendStatus === "OutgoingPending" ? (
              <Button
                variant="outline"
                size="sm"
                onClick={() => void ipc.undoFriend(thread.id, thread.name)}
              >
                Thu hồi lời mời
              </Button>
            ) : null}
          </div>
        ) : null}
      </div>

      {/* Quick action buttons */}
      <div className="border-b border-border">{actionButtons}</div>

      {/* ---- Sections ---- */}

      {/* Group-specific sections: Thành viên nhóm + Bảng tin nhóm */}
      {thread.is_group ? (
        <>
          {/* Thành viên nhóm */}
          <div className="border-b border-border">
            <SectionHeader
              label="Thành viên nhóm"
              expanded={membersExpanded}
              onToggle={() => setMembersExpanded((p) => !p)}
            />
            {membersExpanded ? (
              <div className="flex flex-col px-4 pb-2">
                <button
                  type="button"
                  className="flex cursor-pointer items-center gap-3 py-2 text-foreground hover:text-accent-foreground"
                >
                  <Users className="h-4 w-4 shrink-0 text-muted-foreground" />
                  <span className="text-xs tabular-nums">{memberCount} thành viên</span>
                </button>
                {thread.presentation.members.length > 0 ? (
                  <div className="flex max-h-44 flex-col gap-0.5 overflow-y-auto pt-1 pb-1">
                    {thread.presentation.members.map((m) => (
                      <div key={m.id} className="flex items-center gap-2.5 py-1">
                        <UserAvatar
                          id={m.id}
                          name={m.name}
                          avatarUrl={m.avatar}
                          size={28}
                          mediaCache={mediaCache}
                        />
                        <span className="truncate text-xs text-foreground">{m.name || m.id}</span>
                      </div>
                    ))}
                  </div>
                ) : null}
              </div>
            ) : null}
          </div>

          {/* Bảng tin nhóm */}
          <div className="border-b border-border">
            <SectionHeader
              label="Bảng tin nhóm"
              expanded={boardExpanded}
              onToggle={() => setBoardExpanded((p) => !p)}
            />
            {boardExpanded ? (
              <div className="flex flex-col px-4 pb-2.5">
                <button
                  type="button"
                  className="flex cursor-pointer items-center gap-3 py-2 text-foreground hover:text-accent-foreground"
                >
                  <Clock className="h-4 w-4 shrink-0 text-muted-foreground" />
                  <span className="text-xs">Danh sách nhắc hẹn</span>
                </button>
                <button
                  type="button"
                  className="flex cursor-pointer items-center gap-3 py-2 text-foreground hover:text-accent-foreground"
                >
                  <FileText className="h-4 w-4 shrink-0 text-muted-foreground" />
                  <span className="text-xs">Ghi chú, ghim, bình chọn</span>
                </button>
              </div>
            ) : null}
          </div>
        </>
      ) : (
        <div className="flex flex-col border-b border-border px-4 py-1.5">
          <button
            type="button"
            className="flex cursor-pointer items-center gap-3 py-2 text-foreground hover:text-accent-foreground"
          >
            <Clock className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="text-xs">Danh sách nhắc hẹn</span>
          </button>
        </div>
      )}

      {/* Shared Media Tabs (Ảnh | File | Link) */}
      <Tabs defaultValue="photos" className="flex flex-1 flex-col">
        <div className="border-b border-border p-3">
          <TabsList className="grid w-full grid-cols-3 rounded-md bg-muted p-1">
            <TabsTrigger value="photos" className="py-1 text-xs font-medium">
              Ảnh
            </TabsTrigger>
            <TabsTrigger value="files" className="py-1 text-xs font-medium">
              File
            </TabsTrigger>
            <TabsTrigger value="links" className="py-1 text-xs font-medium">
              Link
            </TabsTrigger>
          </TabsList>
        </div>

        <TabsContent value="photos" className="mt-0 flex-1 p-3">
          {sharedPhotos.length === 0 ? (
            <p className="py-6 text-center text-xs text-muted-foreground">Chưa có hình ảnh nào</p>
          ) : (
            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <span className="text-xs font-medium text-muted-foreground">Ảnh gần đây</span>
                <span className="text-xs text-muted-foreground tabular-nums">
                  {sharedPhotos.length} ảnh
                </span>
              </div>
              <div className="grid grid-cols-3 gap-1.5">
                {sharedPhotos.map((m) => {
                  const key = `img_${m.id || m.ts}`;
                  const fullUrl = m.link_url ?? m.link_thumb ?? "";
                  const thumbSrc = mediaCache[key] ?? jxlToJpgUrl(m.link_thumb ?? m.link_url ?? "");
                  return (
                    <div
                      key={key}
                      onClick={() =>
                        setImageViewerState({
                          texKey: key,
                          fullUrl,
                          title: m.link_title || "Hình ảnh",
                          sender: m.sender,
                          time: m.time,
                        })
                      }
                      className="aspect-square cursor-pointer overflow-hidden rounded border border-border bg-muted text-xs text-muted-foreground hover:opacity-90"
                    >
                      {thumbSrc ? (
                        <img
                          src={thumbSrc}
                          alt="Ảnh"
                          loading="lazy"
                          decoding="async"
                          className="h-full w-full object-cover"
                        />
                      ) : (
                        <div className="flex h-full items-center justify-center">Ảnh</div>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </TabsContent>

        <TabsContent value="files" className="mt-0 flex-1 p-3">
          {sharedFiles.length === 0 ? (
            <p className="py-6 text-center text-xs text-muted-foreground">
              Chưa có file nào được chia sẻ
            </p>
          ) : (
            <div className="flex flex-col gap-2">
              <span className="text-xs font-medium text-muted-foreground">Tệp gần đây</span>
              {sharedFiles.map((m) => (
                <div
                  key={m.id || m.ts}
                  className="flex items-center justify-between gap-2 rounded-md border border-border bg-background p-2"
                >
                  <div className="flex min-w-0 items-center gap-2">
                    <FileText className="h-4 w-4 shrink-0 text-muted-foreground" />
                    <span className="truncate text-xs font-medium text-foreground">
                      {m.link_title || m.text || "Tập tin"}
                    </span>
                  </div>
                  <span className="shrink-0 text-[11px] text-muted-foreground tabular-nums">
                    {m.time}
                  </span>
                </div>
              ))}
            </div>
          )}
        </TabsContent>

        <TabsContent value="links" className="mt-0 flex-1 p-3">
          {sharedLinks.length === 0 ? (
            <p className="py-6 text-center text-xs text-muted-foreground">
              Chưa có liên kết nào được chia sẻ
            </p>
          ) : (
            <div className="flex flex-col gap-2">
              <span className="text-xs font-medium text-muted-foreground">Liên kết gần đây</span>
              {sharedLinks.map((m) => (
                <a
                  key={m.id || m.ts}
                  href={m.link_url ?? "#"}
                  target="_blank"
                  rel="noreferrer"
                  className="flex flex-col gap-0.5 rounded-md border border-border bg-background p-2 transition-colors hover:bg-accent"
                >
                  <span className="truncate text-xs font-medium text-foreground">
                    {m.link_title || m.link_url}
                  </span>
                  <span className="truncate text-[11px] text-muted-foreground">{m.link_url}</span>
                </a>
              ))}
            </div>
          )}
        </TabsContent>
      </Tabs>

      {/* Delete conversation — quiet red text row */}
      <div className="border-t border-border">
        <button
          type="button"
          className="flex w-full cursor-pointer items-center justify-center gap-2 py-3 text-xs font-medium text-destructive hover:bg-destructive/5"
          onClick={() => {
            const last = thread.messages.at(-1);
            void ipc.deleteThread(
              thread.id,
              thread.is_group,
              last?.client_message_id ?? last?.id ?? "",
            );
          }}
        >
          <Trash2 className="h-3.5 w-3.5" />
          Xóa cuộc trò chuyện
        </button>
      </div>
    </aside>
  );
}

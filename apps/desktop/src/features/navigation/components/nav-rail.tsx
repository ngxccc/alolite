import { useState } from "react";
import {
  Briefcase,
  Bug,
  CircleHelp,
  Cloud,
  Database,
  FolderOpen,
  Globe,
  LogOut,
  Megaphone,
  MessageSquare,
  Moon,
  Package,
  Settings,
  Smartphone,
  Star,
  Sun,
  User,
  Users,
  Zap,
} from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/user-avatar";
import { cn } from "cn";
import { useAuthStore, useChatStore, useMediaCacheStore, useUIStore } from "@/stores";
import type { AppSettings } from "@/shared/types/ipc.generated";
import { threadUnreadCount } from "@/features/threads/utils/threads";

// ─── Shared rail button ───────────────────────────────────────────────────────

function RailBtn({
  active = false,
  label,
  onClick,
  children,
  badge,
}: {
  active?: boolean;
  label: string;
  onClick?: () => void;
  children: React.ReactNode;
  badge?: number;
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type="button"
            onClick={onClick}
            aria-label={label}
            aria-current={active ? "page" : undefined}
            className={cn(
              "relative flex h-10 w-10 cursor-pointer items-center justify-center rounded-md transition-colors duration-150 select-none",
              active
                ? "bg-primary text-primary-foreground shadow-sm"
                : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
            )}
          />
        }
      >
        {children}
        {badge != null && badge > 0 && (
          <span className="absolute -top-1 -right-1 inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-destructive px-1 text-[10px] leading-none font-semibold text-destructive-foreground tabular-nums">
            {badge > 99 ? "99+" : badge}
          </span>
        )}
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}

// ─── Settings popup menu ──────────────────────────────────────────────────────

function SettingsMenu({ isDark }: { isDark: boolean }) {
  const settings = useChatStore((s) => s.settings);
  const setSettings = useChatStore((s) => s.setSettings);
  const setIsSyncing = useChatStore((s) => s.setIsSyncing);
  const devMode = useUIStore((s) => s.devMode);
  const setSettingsOpen = useUIStore((s) => s.setSettingsOpen);
  const setPhoneSyncOpen = useUIStore((s) => s.setPhoneSyncOpen);
  const setDevToolsOpen = useUIStore((s) => s.setDevToolsOpen);
  const [open, setOpen] = useState(false);
  const toggleTheme = () => {
    if (!settings) return;
    const next: AppSettings = {
      ...settings,
      theme_mode: isDark ? "light" : "dark",
    };
    setSettings(next);
    void ipc.saveSettings(next);
  };

  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger
          render={
            <DropdownMenuTrigger
              render={
                <button
                  type="button"
                  aria-label="Cài đặt"
                  className={cn(
                    "relative flex h-10 w-10 cursor-pointer items-center justify-center rounded-md transition-colors duration-150 select-none focus-visible:outline-none",
                    open
                      ? "bg-primary text-primary-foreground shadow-sm"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                  )}
                />
              }
            />
          }
        >
          <Settings className="h-5 w-5" />
        </TooltipTrigger>
        <TooltipContent side="right">Cài đặt</TooltipContent>
      </Tooltip>
      <DropdownMenuContent side="right" align="end" sideOffset={8} className="min-w-[196px]">
        {/* Account information */}
        <DropdownMenuItem onClick={() => setSettingsOpen(true, "account")}>
          <User className="h-4 w-4 shrink-0 opacity-70" />
          Thông tin tài khoản
        </DropdownMenuItem>

        {/* General settings */}
        <DropdownMenuItem onClick={() => setSettingsOpen(true, "general")}>
          <Settings className="h-4 w-4 shrink-0 opacity-70" />
          Cài đặt
        </DropdownMenuItem>
        <DropdownMenuSeparator />

        {/* Data sub-menu */}
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>
            <Database className="h-4 w-4 shrink-0 opacity-70" />
            Dữ liệu
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent sideOffset={4}>
            <DropdownMenuItem onClick={() => setIsSyncing(true)}>
              <Cloud className="h-4 w-4 shrink-0 opacity-60" />
              Đồng bộ zCloud
            </DropdownMenuItem>
            <DropdownMenuItem onClick={() => setPhoneSyncOpen(true)}>
              <Smartphone className="h-4 w-4 shrink-0 opacity-60" />
              Đồng bộ tin nhắn từ điện thoại
            </DropdownMenuItem>
          </DropdownMenuSubContent>
        </DropdownMenuSub>

        {/* Language sub-menu */}
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>
            <Globe className="h-4 w-4 shrink-0 opacity-70" />
            Ngôn ngữ
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent sideOffset={4}>
            <DropdownMenuItem disabled>Tiếng Việt (mặc định)</DropdownMenuItem>
          </DropdownMenuSubContent>
        </DropdownMenuSub>

        {/* Help sub-menu */}
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>
            <CircleHelp className="h-4 w-4 shrink-0 opacity-70" />
            Hỗ trợ
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent sideOffset={4}>
            <DropdownMenuItem disabled>Trung tâm hỗ trợ</DropdownMenuItem>
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSeparator />

        {/* Theme toggle */}
        <DropdownMenuItem onClick={toggleTheme}>
          {isDark ? (
            <Sun className="h-4 w-4 shrink-0 opacity-60" />
          ) : (
            <Moon className="h-4 w-4 shrink-0 opacity-60" />
          )}
          {isDark ? "Giao diện sáng" : "Giao diện tối"}
        </DropdownMenuItem>

        {/* Dev tools – only in devMode */}
        {devMode && (
          <DropdownMenuItem onClick={() => setDevToolsOpen(true)}>
            <Bug className="h-4 w-4 shrink-0 opacity-60" />
            Công cụ nhà phát triển
          </DropdownMenuItem>
        )}

        <DropdownMenuSeparator />

        {/* Logout */}
        <DropdownMenuItem variant="destructive" onClick={() => void ipc.logout()}>
          <LogOut className="h-4 w-4 shrink-0 opacity-80" />
          Đăng xuất
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

// ─── zBusiness Tools popover ──────────────────────────────────────────────────

interface ToolItemDef {
  icon: React.ComponentType<{ className?: string }>;
  iconBg: string;
  label: string;
  badge?: "BASIC" | "PRO";
  disabled?: boolean;
  onSelect?: () => void;
}

function ToolItemBtn({ item }: { item: ToolItemDef }) {
  const Icon = item.icon;
  return (
    <button
      type="button"
      disabled={item.disabled}
      onClick={item.onSelect}
      className={cn(
        "flex flex-col items-center gap-1.5 rounded-lg p-2.5 text-center transition-colors",
        item.disabled ? "cursor-not-allowed opacity-50" : "cursor-pointer hover:bg-accent",
      )}
    >
      <div
        className={cn(
          "relative flex h-10 w-10 items-center justify-center rounded-full",
          item.iconBg,
        )}
      >
        <Icon className="h-5 w-5 text-white" />
        {item.badge && (
          <span
            className={cn(
              "absolute -top-1 -right-1 rounded-full px-1 py-0.5 text-xs leading-none font-bold text-white",
              item.badge === "BASIC" ? "bg-sky-500" : "bg-orange-500",
            )}
          >
            {item.badge}
          </span>
        )}
      </div>
      <span className="w-full text-xs leading-tight text-popover-foreground">{item.label}</span>
    </button>
  );
}

function ToolsPopover() {
  const [open, setOpen] = useState(false);
  const setSettingsOpen = useUIStore((s) => s.setSettingsOpen);
  const setActiveSection = useUIStore((s) => s.setActiveSection);
  const toggleFilterStarredOnly = useChatStore((s) => s.toggleFilterStarredOnly);

  const tools: ToolItemDef[] = [
    {
      icon: Zap,
      iconBg: "bg-orange-500",
      label: "Tin nhắn nhanh",
      onSelect: () => {
        setOpen(false);
        setSettingsOpen(true, "messages");
      },
    },
    {
      icon: Package,
      iconBg: "bg-blue-500",
      label: "Danh mục sản phẩm",
      badge: "BASIC",
      disabled: true,
    },
    {
      icon: Zap,
      iconBg: "bg-purple-500",
      label: "Tin nhắn tự động",
      badge: "BASIC",
      disabled: true,
    },
    {
      icon: Star,
      iconBg: "bg-yellow-500",
      label: "Tin nhắn gắn sao",
      onSelect: () => {
        setOpen(false);
        setActiveSection("chat");
        toggleFilterStarredOnly();
      },
    },
    {
      icon: Megaphone,
      iconBg: "bg-sky-600",
      label: "Tin nhắn hàng loạt",
      badge: "PRO",
      disabled: true,
    },
  ];

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <Tooltip>
        <TooltipTrigger
          render={
            <PopoverTrigger
              render={
                <button
                  type="button"
                  aria-label="Công cụ zBusiness"
                  className={cn(
                    "relative flex h-10 w-10 cursor-pointer items-center justify-center rounded-md transition-colors duration-150 select-none",
                    open
                      ? "bg-primary text-primary-foreground shadow-sm"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                  )}
                />
              }
            />
          }
        >
          <Briefcase className="h-5 w-5" />
        </TooltipTrigger>
        <TooltipContent side="right">Công cụ zBusiness</TooltipContent>
      </Tooltip>
      <PopoverContent side="right" align="end" sideOffset={8} className="w-[272px] p-0">
        {/* Popover header */}
        <div className="border-b border-border px-3 py-2.5">
          <span className="text-sm font-semibold text-popover-foreground">Công cụ zBusiness</span>
        </div>
        {/* 3-column tool grid */}
        <div className="grid grid-cols-3 gap-0.5 p-2">
          {tools.map((item) => (
            <ToolItemBtn key={item.label} item={item} />
          ))}
        </div>
      </PopoverContent>
    </Popover>
  );
}

// ─── Main NavRail ─────────────────────────────────────────────────────────────

export function NavRail() {
  const uin = useAuthStore((s) => s.uin);
  const displayName = useAuthStore((s) => s.displayName);
  const avatar = useAuthStore((s) => s.avatar);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);
  const activeSection = useUIStore((s) => s.activeSection);
  const setActiveSection = useUIStore((s) => s.setActiveSection);
  const setSettingsOpen = useUIStore((s) => s.setSettingsOpen);
  const setInChatSearchOpen = useUIStore((s) => s.setInChatSearchOpen);
  const showToast = useUIStore((s) => s.showToast);
  const threads = useChatStore((s) => s.threads);
  const unreadCounts = useChatStore((s) => s.unreadCounts);
  const settings = useChatStore((s) => s.settings);
  const selectThread = useChatStore((s) => s.selectThread);
  const setIsSyncing = useChatStore((s) => s.setIsSyncing);

  const totalUnread = threads.reduce((sum, t) => sum + threadUnreadCount(t, unreadCounts), 0);
  const isDark = (settings?.theme_mode ?? "dark") !== "light";

  const openCloudThread = () => {
    const cloud = threads.find((t) => !t.is_group && t.name.includes("Cloud của tôi"));
    if (cloud) {
      setActiveSection("chat");
      setInChatSearchOpen(false);
      selectThread(cloud.id);
    } else {
      showToast("Chưa tìm thấy hội thoại Cloud của tôi");
    }
  };
  return (
    <aside
      aria-label="Thanh điều hướng chính"
      className="flex h-full w-16 shrink-0 flex-col items-center justify-between border-r border-border bg-rail py-4 text-rail-foreground"
    >
      {/* ── Top group: avatar + main navigation tabs ── */}
      <div className="flex w-full flex-col items-center gap-5">
        {/* User avatar – opens account settings */}
        <Tooltip>
          <TooltipTrigger
            render={
              <button
                type="button"
                onClick={() => setSettingsOpen(true, "account")}
                className="relative cursor-pointer rounded-full ring-1 ring-border transition hover:ring-ring focus-visible:outline-none"
              />
            }
          >
            <UserAvatar
              id={uin || "self"}
              name={displayName || "Me"}
              avatarUrl={avatar}
              size={40}
              mediaCache={mediaCache}
            />
            {/* Online status dot */}
            <span className="absolute right-0 bottom-0 h-2.5 w-2.5 rounded-full bg-online ring-2 ring-rail" />
          </TooltipTrigger>
          <TooltipContent side="right">{displayName || "Tài khoản"}</TooltipContent>
        </Tooltip>

        <nav className="flex w-full flex-col items-center gap-1.5 px-2">
          {/* Messages tab */}
          <RailBtn
            active={activeSection === "chat"}
            label="Tin nhắn"
            onClick={() => setActiveSection("chat")}
            badge={totalUnread}
          >
            <MessageSquare
              className="h-5 w-5"
              strokeWidth={activeSection === "chat" ? 2.25 : 1.75}
            />
          </RailBtn>

          {/* Contacts tab */}
          <RailBtn
            active={activeSection === "contacts"}
            label="Danh bạ"
            onClick={() => setActiveSection("contacts")}
          >
            <Users className="h-5 w-5" strokeWidth={activeSection === "contacts" ? 2.25 : 1.75} />
          </RailBtn>

          {/* My Documents – opens Cloud của tôi thread */}
          <RailBtn label="Cloud của tôi" onClick={openCloudThread}>
            <FolderOpen className="h-5 w-5" strokeWidth={1.75} />
          </RailBtn>
        </nav>
      </div>

      {/* ── Bottom group: utility / settings buttons ── */}
      <div className="flex w-full flex-col items-center gap-1.5 px-2">
        {/* zCloud – trigger cloud sync */}
        <RailBtn label="zCloud" onClick={() => setIsSyncing(true)}>
          <Cloud className="h-5 w-5" strokeWidth={1.75} />
        </RailBtn>

        {/* zBusiness Tools popover */}
        <ToolsPopover />

        {/* Settings dropdown */}
        <SettingsMenu isDark={isDark} />
      </div>
    </aside>
  );
}

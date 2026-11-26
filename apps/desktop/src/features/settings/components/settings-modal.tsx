import type React from "react";
import { useEffect } from "react";
import {
  Cloud,
  Lock,
  MessageSquare,
  Palette,
  Phone,
  Settings,
  Sparkles,
  User,
  Wrench,
  X,
} from "lucide-react";
import { ipc, isTauriRuntime } from "@/shared/api/ipc";
import type { AppSettings } from "@/shared/types/ipc.generated";
import { Button } from "@/shared/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/shared/ui/dialog";
import { cn } from "cn";
import { useAuthStore, useChatStore, useUIStore, type SettingsTab } from "@/stores";
import {
  AccountTab,
  AppearanceTab,
  CallTab,
  GeneralTab,
  MessagesTab,
  PrivacyTab,
  SyncTab,
  UpdateTab,
  UtilitiesTab,
} from "./tabs";
import type { SettingsTabProps } from "./types";

const SETTINGS_TABS: {
  id: SettingsTab;
  label: string;
  icon: typeof Settings;
}[] = [
  { id: "general", label: "Cài đặt chung", icon: Settings },
  { id: "account", label: "Tài khoản", icon: User },
  { id: "privacy", label: "Quyền riêng tư", icon: Lock },
  { id: "call", label: "Cuộc gọi", icon: Phone },
  { id: "sync", label: "Đồng bộ tin nhắn", icon: Cloud },
  { id: "appearance", label: "Giao diện", icon: Palette },
  { id: "messages", label: "Tin nhắn", icon: MessageSquare },
  { id: "utilities", label: "Tiện ích", icon: Wrench },
  { id: "update", label: "Cập nhật", icon: Sparkles },
];

const TAB_COMPONENTS: Record<SettingsTab, React.ComponentType<SettingsTabProps>> = {
  general: GeneralTab,
  account: AccountTab,
  privacy: PrivacyTab,
  call: CallTab,
  sync: SyncTab,
  appearance: AppearanceTab,
  messages: MessagesTab,
  utilities: UtilitiesTab,
  update: UpdateTab,
};

export function SettingsModal() {
  const settingsOpen = useUIStore((s) => s.settingsOpen);
  const activeSettingsTab = useUIStore((s) => s.activeSettingsTab);
  const setSettingsOpen = useUIStore((s) => s.setSettingsOpen);
  const settings = useChatStore((s) => s.settings);
  const setSettings = useChatStore((s) => s.setSettings);
  const setSavedAccounts = useAuthStore((s) => s.setSavedAccounts);

  useEffect(() => {
    if (!settingsOpen || !isTauriRuntime()) return;
    if (activeSettingsTab === "account") {
      void ipc.listSavedAccounts().then((accounts) => setSavedAccounts(accounts));
    }
  }, [settingsOpen, activeSettingsTab, setSavedAccounts]);

  if (!settingsOpen || !settings) return null;

  const updateSettings = (patch: Partial<AppSettings>) => {
    const next: AppSettings = { ...settings, ...patch };
    setSettings(next);
    void ipc.saveSettings(next);
  };

  const ActiveTabContent = TAB_COMPONENTS[activeSettingsTab];

  return (
    <Dialog open={settingsOpen} onOpenChange={(open) => !open && setSettingsOpen(false)}>
      {/* Responsive settings dialog with bounded viewport sizing */}
      <DialogContent
        className="flex h-[min(42rem,calc(100vh-2rem))] w-[min(46rem,calc(100vw-2rem))] max-w-[min(46rem,calc(100vw-2rem))] flex-row gap-0 overflow-hidden rounded-lg p-0"
        showCloseButton={false}
      >
        {/* ── Left nav ── */}
        <nav className="flex w-44 shrink-0 flex-col gap-0.5 overflow-y-auto bg-card px-2 py-4 sm:w-48">
          <DialogTitle className="mb-2 px-2 text-lg font-medium text-foreground">
            Cài đặt
          </DialogTitle>

          {SETTINGS_TABS.map((item) => {
            const Icon = item.icon;
            const active = activeSettingsTab === item.id;
            return (
              <Button
                key={item.id}
                type="button"
                variant="ghost"
                onClick={() => setSettingsOpen(true, item.id)}
                className={cn(
                  "flex h-9 w-full cursor-pointer items-center justify-start gap-2.5 rounded-md px-2.5 text-left text-sm font-normal transition-colors",
                  active ? "bg-accent text-foreground" : "text-foreground hover:bg-white/5",
                )}
              >
                <Icon className="h-4.5 w-4.5 shrink-0 text-muted-foreground" />
                <span className="truncate">{item.label}</span>
              </Button>
            );
          })}
        </nav>

        {/* ── Right body bg-canvas ── */}
        <div className="relative flex flex-1 flex-col overflow-hidden bg-canvas">
          {/* Close X */}
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            onClick={() => setSettingsOpen(false)}
            className="absolute top-3 right-3 z-10 inline-flex h-6 w-6 cursor-pointer items-center justify-center rounded-md text-muted-foreground hover:bg-white/8 hover:text-foreground"
          >
            <X className="h-4.5 w-4.5" />
          </Button>

          <div className="flex-1 overflow-y-auto px-6 py-4">
            <ActiveTabContent settings={settings} updateSettings={updateSettings} />
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

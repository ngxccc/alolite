import { useEffect, useState } from "react";
import { ipc, isTauriRuntime } from "@/shared/api/ipc";
import { ImageViewerModal } from "@/shared/ui/image-viewer-modal";
import { TooltipProvider } from "@/shared/ui/tooltip";
import { LoginView } from "@/features/auth/components/login-view";
import { NavRail } from "@/features/navigation/components/nav-rail";
import { ThreadsSidebar } from "@/features/threads/components/threads-sidebar";
import { InfoDrawer } from "@/features/threads/components/info-drawer";
import { ChatArea } from "@/features/chat/components/chat-area";
import { ForwardModal } from "@/features/chat/components/forward-modal";
import { ContactsView } from "@/features/contacts/components/contacts-view";
import { PhoneLookupModal } from "@/features/contacts/components/phone-lookup-modal";
import { PhoneSyncModal } from "@/features/sync/components/phone-sync-modal";
import { SettingsModal } from "@/features/settings/components/settings-modal";
import { WipeConfirmModal } from "@/features/settings/components/wipe-confirm-modal";
import { DevToolsModal } from "@/features/devtools/components/dev-tools-modal";
import {
  bootstrapStores,
  dispatchCoreEvent,
  useAuthStore,
  useChatStore,
  useMediaCacheStore,
  useUIStore,
} from "@/stores";

export function App() {
  const themeMode = useChatStore((s) => s.settings?.theme_mode ?? "dark");
  const fontSize = useChatStore((s) => s.settings?.font_size ?? "medium");
  const downloadDir = useChatStore((s) => s.settings?.download_dir ?? "");
  const toast = useUIStore((s) => s.toast);
  const clearToast = useUIStore((s) => s.clearToast);
  const setSettingsOpen = useUIStore((s) => s.setSettingsOpen);
  const activeSection = useUIStore((s) => s.activeSection);
  const imageViewer = useUIStore((s) => s.imageViewerState);
  const setImageViewerState = useUIStore((s) => s.setImageViewerState);
  const bootstrapped = useAuthStore((s) => s.bootstrapped);
  const loggedIn = useAuthStore((s) => s.loggedIn);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);
  const [bootError, setBootError] = useState<string | null>(null);

  useEffect(() => {
    const resolved =
      themeMode === "system" && window.matchMedia("(prefers-color-scheme: light)").matches
        ? "light"
        : themeMode === "light"
          ? "light"
          : "dark";
    document.documentElement.setAttribute("data-theme", resolved);
  }, [themeMode]);

  useEffect(() => {
    document.documentElement.setAttribute("data-font-size", fontSize);
  }, [fontSize]);

  useEffect(() => {
    if (!isTauriRuntime()) {
      setBootError("Giao diện cần chạy bên trong cửa sổ ứng dụng Tauri AloLite.");
      return;
    }

    let unlisten: (() => void) | undefined;
    let active = true;

    void ipc
      .onCoreEvent((event) => {
        if (active) dispatchCoreEvent(event);
      })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((err) => setBootError(String(err)));

    void ipc
      .bootstrapState()
      .then((snapshot) => {
        if (active) {
          bootstrapStores(snapshot);
          if (snapshot.logged_in && snapshot.settings.auto_check_updates) {
            void ipc.checkForUpdates(true);
          }
        }
      })
      .catch((err) => setBootError(String(err)));

    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (!toast) return;
    const t = window.setTimeout(() => clearToast(), 2600);
    return () => window.clearTimeout(t);
  }, [toast, clearToast]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "p") {
        e.preventDefault();
        setSettingsOpen(true, "privacy");
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [setSettingsOpen]);

  if (bootError) {
    return (
      <div className="flex h-full w-full items-center justify-center bg-canvas p-6">
        <div className="flex w-full max-w-md flex-col gap-3 rounded-xl border border-border bg-card p-6 shadow-xl">
          <div className="text-base font-bold text-foreground">AloLite</div>
          <div className="text-xs text-destructive">{bootError}</div>
        </div>
      </div>
    );
  }

  if (!bootstrapped) {
    return (
      <div className="flex h-full w-full items-center justify-center bg-canvas">
        <div className="text-sm text-muted-foreground">Đang khởi tạo CoreEngine…</div>
      </div>
    );
  }

  return (
    <TooltipProvider delay={250}>
      {!loggedIn ? (
        <LoginView />
      ) : (
        <div className="flex h-full w-full overflow-hidden bg-background text-foreground">
          <NavRail />
          {activeSection === "contacts" ? (
            <ContactsView />
          ) : (
            <>
              <ThreadsSidebar />
              <ChatArea />
              <InfoDrawer />
            </>
          )}
        </div>
      )}

      <SettingsModal />
      <PhoneLookupModal />
      <PhoneSyncModal />
      <ForwardModal />
      <WipeConfirmModal />
      <DevToolsModal />
      <ImageViewerModal
        viewer={imageViewer}
        mediaCache={mediaCache}
        downloadDir={downloadDir}
        onClose={() => setImageViewerState(null)}
      />

      {toast ? (
        <div className="fixed right-4 bottom-4 z-50 flex items-center gap-2 rounded-lg border border-border bg-popover px-3.5 py-2 text-xs font-medium text-popover-foreground shadow-xl">
          <span className="h-2 w-2 shrink-0 rounded-full bg-online" />
          <span>{toast.message}</span>
        </div>
      ) : null}
    </TooltipProvider>
  );
}

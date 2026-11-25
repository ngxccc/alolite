import { useState } from "react";
import { RefreshCw, Trash2 } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { useChatStore, useUIStore } from "@/stores";
import { SectionHeading, SettingCard } from "../primitives";

export function SyncTab() {
  const activeDbPath = useUIStore((s) => s.activeDbPath);
  const setPhoneSyncOpen = useUIStore((s) => s.setPhoneSyncOpen);
  const setWipeConfirmOpen = useUIStore((s) => s.setWipeConfirmOpen);
  const showToast = useUIStore((s) => s.showToast);
  const isSyncing = useChatStore((s) => s.isSyncing);
  const setIsSyncing = useChatStore((s) => s.setIsSyncing);
  const threads = useChatStore((s) => s.threads);
  const [migratePath, setMigratePath] = useState("");

  return (
    <>
      {/* Illustration card matching set_sync.png */}
      <div className="mb-4 flex flex-col items-center justify-center rounded-lg bg-card py-8">
        <div className="mb-3 flex items-center gap-3">
          {/* phone icon */}
          <div className="flex h-10 w-10 items-center justify-center rounded-full border-2 border-border">
            <svg
              width="20"
              height="20"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              className="text-primary"
            >
              <rect x="5" y="2" width="14" height="20" rx="2" />
              <line x1="12" y1="18" x2="12" y2="18.01" strokeWidth="2" strokeLinecap="round" />
            </svg>
          </div>
          <div className="flex gap-1">
            <div className="h-1.5 w-1.5 rounded-full bg-primary/40" />
            <div className="h-1.5 w-1.5 rounded-full bg-primary/60" />
            <div className="h-1.5 w-1.5 rounded-full bg-primary" />
          </div>
          {/* PC icon */}
          <div className="flex h-10 w-10 items-center justify-center rounded-full border-2 border-primary bg-primary/10">
            <svg
              width="20"
              height="20"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.5"
              className="text-primary"
            >
              <rect x="2" y="3" width="20" height="14" rx="2" />
              <line x1="8" y1="21" x2="16" y2="21" strokeWidth="2" strokeLinecap="round" />
              <line x1="12" y1="17" x2="12" y2="21" strokeWidth="2" strokeLinecap="round" />
            </svg>
          </div>
        </div>
        <div className="text-sm font-medium text-foreground">Đồng bộ tin nhắn</div>
        <div className="mt-1 text-xs text-muted-foreground">
          Đồng bộ tin nhắn từ điện thoại sang Zalo PC
        </div>
      </div>

      <SettingCard>
        <div className="px-4 py-3">
          <div className="mb-2 text-xs text-muted-foreground">
            CSDL đang dùng:{" "}
            <code className="rounded bg-muted px-1.5 py-0.5 text-foreground">{activeDbPath}</code>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="default" size="sm" onClick={() => setPhoneSyncOpen(true)}>
              Đồng bộ từ điện thoại
            </Button>
            {isSyncing ? (
              <Button variant="destructive" size="sm" onClick={() => void ipc.cancelCloudSync()}>
                Dừng đồng bộ Nhóm/Cloud
              </Button>
            ) : (
              <Button
                variant="outline"
                size="sm"
                onClick={() => {
                  const targets = threads
                    .filter((t) => t.is_group || t.name.includes("Cloud của tôi"))
                    .map<[string, string, boolean]>((t) => [t.id, t.name, t.is_group]);
                  setIsSyncing(true);
                  void ipc.syncHistory(targets);
                }}
              >
                <RefreshCw className="h-3.5 w-3.5" /> Đồng bộ Nhóm &amp; Cloud
              </Button>
            )}
          </div>
        </div>
      </SettingCard>

      <SectionHeading>Nhập lịch sử từ tệp sao lưu</SectionHeading>
      <SettingCard>
        <div className="px-4 py-3">
          <div className="mb-2 flex gap-1.5">
            <Input
              value={migratePath}
              onChange={(e) => setMigratePath(e.target.value)}
              placeholder="Đường dẫn file hoặc thư mục Zalo…"
            />
            <Button
              variant="outline"
              onClick={async () => {
                const p = await ipc.pickFile({
                  title: "Chọn file hoặc thư mục dữ liệu Zalo",
                });
                if (p) setMigratePath(p);
              }}
            >
              Duyệt…
            </Button>
            <Button
              variant="default"
              onClick={async () => {
                if (!migratePath.trim()) return;
                try {
                  await ipc.importLocalHistory(migratePath.trim());
                } catch (e) {
                  showToast(String(e), "error");
                }
              }}
            >
              Nhập ngay
            </Button>
          </div>
          <Button
            variant="outline"
            size="sm"
            className="w-fit"
            onClick={async () => {
              try {
                await ipc.autoDetectImportHistory();
              } catch (e) {
                showToast(String(e), "error");
              }
            }}
          >
            Tự động quét dữ liệu Zalo trên máy
          </Button>
        </div>
      </SettingCard>

      <SectionHeading>Xóa dữ liệu cục bộ</SectionHeading>
      <SettingCard>
        <div className="flex items-center justify-between px-4 py-3">
          <div>
            <div className="text-sm font-medium text-destructive">Xóa dữ liệu tin nhắn cục bộ</div>
            <div className="text-xs text-muted-foreground">
              Giữ nguyên phiên đăng nhập session.json.
            </div>
          </div>
          <Button variant="destructive" size="sm" onClick={() => setWipeConfirmOpen(true)}>
            <Trash2 className="h-3.5 w-3.5" /> Xóa dữ liệu
          </Button>
        </div>
      </SettingCard>
    </>
  );
}

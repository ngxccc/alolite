import { useState } from "react";
import { Forward, Search } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import { Button } from "@/shared/ui/button";
import { UserAvatar } from "@/shared/ui/user-avatar";
import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { matchesVnQuery } from "@/shared/utils/text";
import { useChatStore, useMediaCacheStore, useUIStore } from "@/stores";

export function ForwardModal() {
  const forwardMsg = useChatStore((s) => s.forwardMsg);
  const setForwardMsg = useChatStore((s) => s.setForwardMsg);
  const threads = useChatStore((s) => s.threads);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);
  const showToast = useUIStore((s) => s.showToast);
  const [filter, setFilter] = useState("");

  if (!forwardMsg) return null;

  const preview = forwardMsg.sticker_id ? "[Nhãn dán]" : forwardMsg.text.slice(0, 80);
  const candidates = threads.filter((t) => matchesVnQuery(t.name, filter));

  return (
    <Dialog open={Boolean(forwardMsg)} onOpenChange={(open) => !open && setForwardMsg(null)}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-base font-semibold">
            <Forward className="h-4.5 w-4.5 text-primary" /> Chuyển tiếp tin nhắn
          </DialogTitle>
        </DialogHeader>

        <div className="flex flex-col gap-3 p-4">
          <div className="truncate rounded-md bg-muted px-3 py-2 text-sm text-muted-foreground">
            Nội dung: {preview}
          </div>

          <div className="relative flex items-center">
            <Search className="pointer-events-none absolute left-3 h-4 w-4 text-muted-foreground" />
            <Input
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder="Tìm hội thoại…"
              className="h-9 pl-9 text-sm"
              autoFocus
            />
          </div>

          <div className="flex max-h-72 flex-col gap-0.5 overflow-y-auto">
            {candidates.map((t) => (
              <Button
                key={t.id}
                type="button"
                variant="ghost"
                onClick={() => {
                  void ipc.forwardMessage(t.id, t.is_group, forwardMsg);
                  setForwardMsg(null);
                  showToast(`Đang chuyển tiếp tới ${t.name}`);
                }}
                className="flex h-auto cursor-pointer items-center justify-start gap-3 rounded-md px-2.5 py-2 text-left text-sm font-medium transition hover:bg-accent hover:text-accent-foreground"
              >
                <UserAvatar
                  id={t.id}
                  name={t.name}
                  avatarUrl={t.avatar}
                  isGroup={t.is_group}
                  size={36}
                  mediaCache={mediaCache}
                />
                <span className="truncate">{t.is_group ? `[Nhóm] ${t.name}` : t.name}</span>
              </Button>
            ))}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

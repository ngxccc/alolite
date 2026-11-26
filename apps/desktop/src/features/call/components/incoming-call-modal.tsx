import { Phone, PhoneOff } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { UserAvatar } from "@/shared/ui/user-avatar";
import { useCallStore, useMediaCacheStore, useUIStore } from "@/stores";

export function IncomingCallModal() {
  const phase = useCallStore((s) => s.phase);
  const peerId = useCallStore((s) => s.peerId);
  const peerName = useCallStore((s) => s.peerName);
  const peerAvatar = useCallStore((s) => s.peerAvatar);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);
  const showToast = useUIStore((s) => s.showToast);

  const open = phase === "incoming_ringing";
  const displayName = peerName || peerId || "Người dùng Zalo";

  const handleAnswer = async (accept: boolean) => {
    try {
      await ipc.answerCall(accept);
    } catch (err) {
      showToast(`Không thể phản hồi cuộc gọi: ${String(err)}`, "error");
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && open) {
          void handleAnswer(false);
        }
      }}
    >
      <DialogContent className="max-w-sm" showCloseButton={false}>
        <DialogHeader className="items-center text-center">
          <div className="mb-2 flex justify-center">
            <UserAvatar
              id={peerId ?? "incoming-peer"}
              name={displayName}
              avatarUrl={peerAvatar}
              isGroup={false}
              size={64}
              mediaCache={mediaCache}
            />
          </div>
          <DialogTitle className="text-base font-semibold text-foreground">
            {displayName}
          </DialogTitle>
          <DialogDescription className="text-xs text-muted-foreground">
            Cuộc gọi thoại đến · Đang đổ chuông…
          </DialogDescription>
        </DialogHeader>

        <div className="mt-4 flex items-center justify-center gap-4">
          <Button
            type="button"
            variant="destructive"
            className="gap-2"
            onClick={() => void handleAnswer(false)}
          >
            <PhoneOff className="h-4 w-4" aria-hidden="true" />
            <span>Từ chối</span>
          </Button>
          <Button
            type="button"
            variant="default"
            className="gap-2 bg-online text-primary-foreground hover:bg-online/90"
            onClick={() => void handleAnswer(true)}
          >
            <Phone className="h-4 w-4" aria-hidden="true" />
            <span>Nghe máy</span>
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

import { useEffect, useState } from "react";
import { Mic, MicOff, PhoneCall, PhoneOff } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import { Button } from "@/shared/ui/button";
import { UserAvatar } from "@/shared/ui/user-avatar";
import { useCallStore, useMediaCacheStore, useUIStore } from "@/stores";

export function formatCallDuration(totalSeconds: number): string {
  const safe = Math.max(0, Math.floor(totalSeconds));
  const mins = Math.floor(safe / 60);
  const secs = safe % 60;
  return `${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
}

export function ActiveCallBar() {
  const phase = useCallStore((s) => s.phase);
  const peerId = useCallStore((s) => s.peerId);
  const peerName = useCallStore((s) => s.peerName);
  const peerAvatar = useCallStore((s) => s.peerAvatar);
  const isMuted = useCallStore((s) => s.isMuted);
  const startedAtMs = useCallStore((s) => s.startedAtMs);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);
  const showToast = useUIStore((s) => s.showToast);

  const [elapsedSecs, setElapsedSecs] = useState(0);

  useEffect(() => {
    if (phase !== "active" || startedAtMs == null) {
      setElapsedSecs(0);
      return;
    }
    const update = () => {
      setElapsedSecs(Math.max(0, Math.floor((Date.now() - startedAtMs) / 1000)));
    };
    update();
    const timer = window.setInterval(update, 1000);
    return () => window.clearInterval(timer);
  }, [phase, startedAtMs]);

  if (phase === "idle" || phase === "incoming_ringing") {
    return null;
  }

  const displayName = peerName || peerId || "Người dùng Zalo";
  const statusText =
    phase === "outgoing_ringing"
      ? "Đang đổ chuông…"
      : phase === "connecting"
        ? "Đang kết nối…"
        : phase === "ending"
          ? "Đang kết thúc…"
          : formatCallDuration(elapsedSecs);

  const handleToggleMute = async () => {
    try {
      await ipc.setCallMuted(!isMuted);
    } catch (err) {
      showToast(`Không thể đổi trạng thái mic: ${String(err)}`, "error");
    }
  };

  const handleEndCall = async () => {
    try {
      await ipc.endCall();
    } catch (err) {
      showToast(`Không thể kết thúc cuộc gọi: ${String(err)}`, "error");
    }
  };

  return (
    <div
      role="region"
      aria-label="Thanh cuộc gọi đang hoạt động"
      className="fixed top-3 left-1/2 z-40 flex -translate-x-1/2 items-center gap-3 rounded-xl border border-border bg-card px-4 py-2 shadow-xl"
    >
      <div className="flex items-center gap-2.5">
        <UserAvatar
          id={peerId ?? "active-peer"}
          name={displayName}
          avatarUrl={peerAvatar}
          isGroup={false}
          size={32}
          mediaCache={mediaCache}
        />
        <div className="min-w-0">
          <div className="max-w-44 truncate text-xs font-semibold text-foreground">
            {displayName}
          </div>
          <div className="flex items-center gap-1 text-[11px] font-medium text-online tabular-nums">
            <PhoneCall className="h-3 w-3 shrink-0" aria-hidden="true" />
            <span>{statusText}</span>
          </div>
        </div>
      </div>

      <div className="ml-2 flex items-center gap-1.5">
        <Button
          type="button"
          variant={isMuted ? "secondary" : "ghost"}
          size="icon"
          aria-label={isMuted ? "Bật mic" : "Tắt mic"}
          aria-pressed={isMuted}
          title={isMuted ? "Bật mic" : "Tắt mic"}
          className="h-8 w-8"
          disabled={phase !== "active"}
          onClick={() => void handleToggleMute()}
        >
          {isMuted ? (
            <MicOff className="h-4 w-4 text-destructive" aria-hidden="true" />
          ) : (
            <Mic className="h-4 w-4" aria-hidden="true" />
          )}
        </Button>

        <Button
          type="button"
          variant="destructive"
          size="sm"
          aria-label="Kết thúc cuộc gọi"
          className="h-8 gap-1.5 px-3"
          onClick={() => void handleEndCall()}
        >
          <PhoneOff className="h-3.5 w-3.5" aria-hidden="true" />
          <span>Kết thúc</span>
        </Button>
      </div>
    </div>
  );
}

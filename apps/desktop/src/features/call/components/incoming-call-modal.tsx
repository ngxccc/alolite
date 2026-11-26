import { useEffect, useRef } from "react";
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
import { useCallStore, useChatStore, useMediaCacheStore, useUIStore } from "@/stores";

const noop = (): void => undefined;

function getAudioContextCtor(): typeof AudioContext | undefined {
  if (typeof window === "undefined") {
    return undefined;
  }
  const win = window as unknown as {
    AudioContext?: typeof AudioContext;
    webkitAudioContext?: typeof AudioContext;
  };
  const ctor = win.AudioContext ?? win.webkitAudioContext;
  return typeof ctor === "function" ? ctor : undefined;
}

export interface RingtoneOptions {
  enabled?: boolean;
  volumePct?: number;
  singleBurst?: boolean;
}

export function startIncomingRingtone(options?: RingtoneOptions): () => void {
  if (options?.enabled === false) {
    return noop;
  }
  const clampedVol = Math.max(0, Math.min(100, options?.volumePct ?? 80));
  if (clampedVol === 0) {
    return noop;
  }
  const AudioCtx = getAudioContextCtor();
  if (!AudioCtx) {
    return noop;
  }

  let ctx: AudioContext;
  try {
    ctx = new AudioCtx();
  } catch {
    return noop;
  }

  let stopped = false;
  const peakGain = 0.08 * (clampedVol / 100);
  const playBurst = () => {
    if (stopped || ctx.state === "closed") {
      return;
    }
    try {
      const now = ctx.currentTime;
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = "sine";
      osc.frequency.setValueAtTime(480, now);
      osc.frequency.setValueAtTime(540, now + 0.25);

      gain.gain.setValueAtTime(0.0001, now);
      gain.gain.exponentialRampToValueAtTime(Math.max(0.0002, peakGain), now + 0.04);
      gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.55);

      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.start(now);
      osc.stop(now + 0.58);
    } catch {
      // Best-effort ringtone playback; ignore Web Audio autoplay restrictions.
    }
  };

  playBurst();
  if (options?.singleBurst) {
    const closeTimer = window.setTimeout(() => {
      stopped = true;
      if (ctx.state !== "closed") {
        void ctx.close().catch(noop);
      }
    }, 650);
    return () => {
      stopped = true;
      window.clearTimeout(closeTimer);
      if (ctx.state !== "closed") {
        void ctx.close().catch(noop);
      }
    };
  }
  const intervalId = window.setInterval(playBurst, 2200);

  return () => {
    stopped = true;
    window.clearInterval(intervalId);
    if (ctx.state !== "closed") {
      void ctx.close().catch(noop);
    }
  };
}

export function IncomingCallModal() {
  const phase = useCallStore((s) => s.phase);
  const peerId = useCallStore((s) => s.peerId);
  const peerName = useCallStore((s) => s.peerName);
  const peerAvatar = useCallStore((s) => s.peerAvatar);
  const ringtoneEnabled = useChatStore((s) => s.settings?.call_ringtone_enabled ?? true);
  const ringtoneVolumePct = useChatStore((s) => s.settings?.call_ringtone_volume_percent ?? 80);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);
  const showToast = useUIStore((s) => s.showToast);
  const stopRingtoneRef = useRef<(() => void) | null>(null);

  const open = phase === "incoming_ringing";
  const displayName = peerName || peerId || "Người dùng Zalo";

  useEffect(() => {
    if (!open) {
      stopRingtoneRef.current?.();
      stopRingtoneRef.current = null;
      return;
    }
    const stop = startIncomingRingtone({
      enabled: ringtoneEnabled,
      volumePct: ringtoneVolumePct,
    });
    stopRingtoneRef.current = stop;
    return () => {
      stop();
      if (stopRingtoneRef.current === stop) {
        stopRingtoneRef.current = null;
      }
    };
  }, [open, ringtoneEnabled, ringtoneVolumePct]);

  const handleAnswer = async (accept: boolean) => {
    stopRingtoneRef.current?.();
    stopRingtoneRef.current = null;
    try {
      await ipc.answerCall(accept);
    } catch (err) {
      console.error("Failed to answer incoming call:", err);
      showToast("Không thể phản hồi cuộc gọi", "error");
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

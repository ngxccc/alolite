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
  const peakGain = 0.12 * (clampedVol / 100);
  // Gentle 4-note chime motif (G5 - E5 - G5 - C6) with soft attack/decay.
  const notes = [
    { freq: 783.99, offset: 0.0, dur: 0.2 },
    { freq: 659.25, offset: 0.18, dur: 0.2 },
    { freq: 783.99, offset: 0.36, dur: 0.2 },
    { freq: 1046.5, offset: 0.54, dur: 0.42 },
  ];
  const playBurst = () => {
    if (stopped || ctx.state === "closed") {
      return;
    }
    try {
      const base = ctx.currentTime;
      for (const n of notes) {
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        const t0 = base + n.offset;
        osc.type = "triangle";
        osc.frequency.setValueAtTime(n.freq, t0);

        gain.gain.setValueAtTime(0.0001, t0);
        gain.gain.exponentialRampToValueAtTime(Math.max(0.0002, peakGain), t0 + 0.025);
        gain.gain.exponentialRampToValueAtTime(0.0001, t0 + n.dur);

        osc.connect(gain);
        gain.connect(ctx.destination);
        osc.start(t0);
        osc.stop(t0 + n.dur + 0.02);
      }
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
    }, 1100);
    return () => {
      stopped = true;
      window.clearTimeout(closeTimer);
      if (ctx.state !== "closed") {
        void ctx.close().catch(noop);
      }
    };
  }
  const intervalId = window.setInterval(playBurst, 2500);

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

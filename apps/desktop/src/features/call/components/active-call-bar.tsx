import { useEffect, useState } from "react";
import {
  Mic,
  MicOff,
  Pause,
  PhoneCall,
  PhoneOff,
  Play,
  SlidersHorizontal,
  Volume2,
} from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import type { AudioDeviceList, CallPhase } from "@/shared/types/ipc.generated";
import { Button } from "@/shared/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "@/shared/ui/popover";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { UserAvatar } from "@/shared/ui/user-avatar";
import { formatCallDuration } from "@/shared/utils/text";
import { useCallStore, useMediaCacheStore, useUIStore } from "@/stores";

export function getCallStatusText(phase: CallPhase, elapsedSecs: number, isHeld: boolean): string {
  if (phase === "outgoing_ringing") {
    return "Đang đổ chuông…";
  }
  if (phase === "connecting") {
    return "Đang kết nối…";
  }
  if (phase === "ending") {
    return "Đang kết thúc…";
  }
  if (phase === "active" && isHeld) {
    return "Đang tạm giữ";
  }
  return formatCallDuration(elapsedSecs);
}

export function ActiveCallBar() {
  const phase = useCallStore((s) => s.phase);
  const peerId = useCallStore((s) => s.peerId);
  const peerName = useCallStore((s) => s.peerName);
  const peerAvatar = useCallStore((s) => s.peerAvatar);
  const isMuted = useCallStore((s) => s.isMuted);
  const isHeld = useCallStore((s) => s.isHeld);
  const startedAtMs = useCallStore((s) => s.startedAtMs);
  const selectedMicId = useCallStore((s) => s.selectedMicId);
  const selectedSpeakerId = useCallStore((s) => s.selectedSpeakerId);
  const setSelectedDevices = useCallStore((s) => s.setSelectedDevices);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);
  const showToast = useUIStore((s) => s.showToast);

  const [elapsedSecs, setElapsedSecs] = useState(0);
  const [devicesOpen, setDevicesOpen] = useState(false);
  const [loadingDevices, setLoadingDevices] = useState(false);
  const [devices, setDevices] = useState<AudioDeviceList>({ inputs: [], outputs: [] });

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
  const statusText = getCallStatusText(phase, elapsedSecs, isHeld);

  const handleToggleMute = async () => {
    try {
      await ipc.setCallMuted(!isMuted);
    } catch (err) {
      console.error("Failed to toggle call mute:", err);
      showToast("Không thể đổi trạng thái mic", "error");
    }
  };

  const handleToggleHold = async () => {
    try {
      await ipc.setCallHeld(!isHeld);
    } catch (err) {
      console.error("Failed to toggle call hold:", err);
      showToast("Không thể đổi trạng thái tạm giữ", "error");
    }
  };

  const handleDevicesPopoverChange = async (open: boolean) => {
    setDevicesOpen(open);
    if (!open) {
      return;
    }
    setLoadingDevices(true);
    try {
      const list = await ipc.listAudioDevices();
      setDevices(list);
    } catch (err) {
      console.error("Failed to list audio devices:", err);
      showToast("Không thể tải danh sách thiết bị âm thanh", "error");
    } finally {
      setLoadingDevices(false);
    }
  };

  const handleMicChange = async (micId: string | null) => {
    const nextMic = micId ?? null;
    setSelectedDevices(nextMic, selectedSpeakerId);
    try {
      await ipc.setCallDevices(nextMic, selectedSpeakerId);
    } catch (err) {
      console.error("Failed to change microphone:", err);
      showToast("Không thể đổi micro", "error");
    }
  };

  const handleSpeakerChange = async (speakerId: string | null) => {
    const nextSpeaker = speakerId ?? null;
    setSelectedDevices(selectedMicId, nextSpeaker);
    try {
      await ipc.setCallDevices(selectedMicId, nextSpeaker);
    } catch (err) {
      console.error("Failed to change speaker:", err);
      showToast("Không thể đổi loa", "error");
    }
  };

  const handleEndCall = async () => {
    try {
      await ipc.endCall();
    } catch (err) {
      console.error("Failed to end call:", err);
      showToast("Không thể kết thúc cuộc gọi", "error");
    }
  };

  const activeMicValue =
    selectedMicId ??
    devices.inputs.find((d) => d.is_default)?.id ??
    devices.inputs[0]?.id ??
    "default";
  const activeSpeakerValue =
    selectedSpeakerId ??
    devices.outputs.find((d) => d.is_default)?.id ??
    devices.outputs[0]?.id ??
    "default";

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
          <div
            className={`flex items-center gap-1 text-[11px] font-medium tabular-nums ${
              isHeld ? "text-warning" : "text-online"
            }`}
          >
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
          disabled={phase !== "active" && phase !== "connecting"}
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
          variant={isHeld ? "secondary" : "ghost"}
          size="icon"
          aria-label={isHeld ? "Tiếp tục cuộc gọi" : "Tạm giữ cuộc gọi"}
          aria-pressed={isHeld}
          title={isHeld ? "Tiếp tục cuộc gọi" : "Tạm giữ cuộc gọi"}
          className="h-8 w-8"
          disabled={phase !== "active"}
          onClick={() => void handleToggleHold()}
        >
          {isHeld ? (
            <Play className="h-4 w-4 text-warning" aria-hidden="true" />
          ) : (
            <Pause className="h-4 w-4" aria-hidden="true" />
          )}
        </Button>

        <Popover open={devicesOpen} onOpenChange={(open) => void handleDevicesPopoverChange(open)}>
          <PopoverTrigger
            render={
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label="Chọn thiết bị âm thanh"
                title="Thiết bị âm thanh"
                className="h-8 w-8"
              />
            }
          >
            <SlidersHorizontal className="h-4 w-4" aria-hidden="true" />
          </PopoverTrigger>
          <PopoverContent align="end" className="w-72 gap-3 p-3">
            <PopoverHeader>
              <PopoverTitle className="text-xs font-semibold">Thiết bị âm thanh</PopoverTitle>
            </PopoverHeader>
            {loadingDevices ? (
              <p className="text-xs text-muted-foreground">Đang tải thiết bị…</p>
            ) : (
              <div className="flex flex-col gap-2.5">
                <div className="flex flex-col gap-1">
                  <span className="flex items-center gap-1.5 text-[11px] font-medium text-muted-foreground">
                    <Mic className="h-3 w-3" aria-hidden="true" />
                    Micro
                  </span>
                  <Select value={activeMicValue} onValueChange={(val) => void handleMicChange(val)}>
                    <SelectTrigger size="sm" className="w-full" aria-label="Chọn micro">
                      <SelectValue placeholder="Micro mặc định" />
                    </SelectTrigger>
                    <SelectContent>
                      {devices.inputs.map((dev) => (
                        <SelectItem key={dev.id} value={dev.id}>
                          {dev.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                <div className="flex flex-col gap-1">
                  <span className="flex items-center gap-1.5 text-[11px] font-medium text-muted-foreground">
                    <Volume2 className="h-3 w-3" aria-hidden="true" />
                    Loa
                  </span>
                  <Select
                    value={activeSpeakerValue}
                    onValueChange={(val) => void handleSpeakerChange(val)}
                  >
                    <SelectTrigger size="sm" className="w-full" aria-label="Chọn loa">
                      <SelectValue placeholder="Loa mặc định" />
                    </SelectTrigger>
                    <SelectContent>
                      {devices.outputs.map((dev) => (
                        <SelectItem key={dev.id} value={dev.id}>
                          {dev.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>
            )}
          </PopoverContent>
        </Popover>

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

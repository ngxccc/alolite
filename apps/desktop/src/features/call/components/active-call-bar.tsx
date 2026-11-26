import { useEffect, useState } from "react";
import { Mic, MicOff, PhoneCall, PhoneOff, SlidersHorizontal, Volume2 } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import type { AudioDeviceList, CallPhase } from "@/shared/types/ipc.generated";
import { Button } from "@/shared/ui/button";
import { Label } from "@/shared/ui/label";
import {
  Popover,
  PopoverContent,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
} from "@/shared/ui/popover";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { Slider } from "@/shared/ui/slider";
import { Switch } from "@/shared/ui/switch";
import { UserAvatar } from "@/shared/ui/user-avatar";
import { formatCallDuration } from "@/shared/utils/text";
import { useCallStore, useMediaCacheStore, useUIStore } from "@/stores";

export function getCallStatusText(phase: CallPhase, elapsedSecs: number, isMuted = false): string {
  if (phase === "outgoing_ringing") {
    return "Đang đổ chuông…";
  }
  if (phase === "connecting") {
    return "Đang kết nối…";
  }
  if (phase === "ending") {
    return "Đang kết thúc…";
  }
  const duration = formatCallDuration(elapsedSecs);
  if (phase === "active" && isMuted) {
    return `Mic đang tắt · ${duration}`;
  }
  return `Đang trò chuyện · ${duration}`;
}

export function ActiveCallBar() {
  const phase = useCallStore((s) => s.phase);
  const peerId = useCallStore((s) => s.peerId);
  const peerName = useCallStore((s) => s.peerName);
  const peerAvatar = useCallStore((s) => s.peerAvatar);
  const isIncoming = useCallStore((s) => s.isIncoming);
  const isMuted = useCallStore((s) => s.isMuted);
  const peerMuted = useCallStore((s) => s.peerMuted);
  const startedAtMs = useCallStore((s) => s.startedAtMs);
  const selectedMicId = useCallStore((s) => s.selectedMicId);
  const selectedSpeakerId = useCallStore((s) => s.selectedSpeakerId);
  const autoMic = useCallStore((s) => s.autoMic);
  const autoSpeaker = useCallStore((s) => s.autoSpeaker);
  const micVolume = useCallStore((s) => s.micVolume);
  const speakerVolume = useCallStore((s) => s.speakerVolume);
  const micLevel = useCallStore((s) => s.micLevel);
  const speakerLevel = useCallStore((s) => s.speakerLevel);
  const micGainPct = useCallStore((s) => s.micGainPct);
  const speakerGainPct = useCallStore((s) => s.speakerGainPct);
  const setSelectedDevices = useCallStore((s) => s.setSelectedDevices);
  const setAudioConfig = useCallStore((s) => s.setAudioConfig);
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

  const _directionHint = isIncoming ? "Cuộc gọi đến" : "Cuộc gọi đi";
  const displayName = peerName || peerId || "Người dùng Zalo";
  const statusText = getCallStatusText(phase, elapsedSecs, isMuted);

  const handleToggleMute = async () => {
    try {
      await ipc.setCallMuted(!isMuted);
    } catch (err) {
      console.error("Failed to toggle call mute:", err);
      showToast("Không thể đổi trạng thái mic", "error");
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

  const handleAudioConfigChange = async (
    nextAutoMic: boolean,
    nextAutoSpeaker: boolean,
    nextMic: number,
    nextSpeaker: number,
  ) => {
    setAudioConfig(nextAutoMic, nextAutoSpeaker, nextMic, nextSpeaker);
    try {
      await ipc.setCallAudioConfig(nextAutoMic, nextAutoSpeaker, nextMic, nextSpeaker);
    } catch (err) {
      console.error("Failed to update call audio config:", err);
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

  const selectedMicDev =
    devices.inputs.find((d) => d.id === selectedMicId) ??
    devices.inputs.find((d) => d.is_default) ??
    devices.inputs[0];
  const selectedSpeakerDev =
    devices.outputs.find((d) => d.id === selectedSpeakerId) ??
    devices.outputs.find((d) => d.is_default) ??
    devices.outputs[0];

  const activeMicValue = selectedMicDev?.id ?? "default";
  const activeSpeakerValue = selectedSpeakerDev?.id ?? "default";

  return (
    <div
      role="region"
      aria-label="Thanh cuộc gọi đang hoạt động"
      title={_directionHint}
      className="fixed top-3 left-1/2 z-40 flex w-max max-w-[calc(100vw-2rem)] -translate-x-1/2 items-center gap-3 rounded-xl border border-border bg-card px-4 py-2.5 shadow-xl"
    >
      <div className="flex min-w-0 items-center gap-2.5">
        <UserAvatar
          id={peerId ?? "active-peer"}
          name={displayName}
          avatarUrl={peerAvatar}
          isGroup={false}
          size={36}
          mediaCache={mediaCache}
        />
        <div className="min-w-0">
          <div className="max-w-56 truncate text-xs font-semibold whitespace-nowrap text-foreground">
            {displayName}
          </div>
          <div
            className={`flex items-center gap-1.5 text-[11px] font-medium whitespace-nowrap tabular-nums ${
              isMuted ? "text-destructive" : "text-online"
            }`}
          >
            <PhoneCall className="h-3 w-3 shrink-0" aria-hidden="true" />
            <span>{statusText}</span>
            {peerMuted ? (
              <span className="inline-flex items-center gap-1 rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                <MicOff className="h-2.5 w-2.5" aria-hidden="true" />
                Đối phương tắt mic
              </span>
            ) : null}
          </div>
        </div>
      </div>

      <div className="ml-2 flex shrink-0 items-center gap-1.5">
        <Button
          type="button"
          variant={isMuted ? "secondary" : "ghost"}
          size="sm"
          aria-label={isMuted ? "Bật mic" : "Tắt mic"}
          aria-pressed={isMuted}
          title={isMuted ? "Bật mic" : "Tắt mic"}
          className="h-8 gap-1.5 px-2.5 text-xs"
          disabled={phase !== "active" && phase !== "connecting"}
          onClick={() => void handleToggleMute()}
        >
          {isMuted ? (
            <MicOff className="h-3.5 w-3.5 text-destructive" aria-hidden="true" />
          ) : (
            <Mic className="h-3.5 w-3.5" aria-hidden="true" />
          )}
          <span className="hidden md:inline">{isMuted ? "Bật mic" : "Tắt mic"}</span>
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
          <PopoverContent
            align="end"
            className="w-auto max-w-[min(36rem,calc(100vw-2rem))] min-w-80 gap-3 p-3.5"
          >
            <PopoverHeader>
              <PopoverTitle className="text-xs font-semibold">Thiết bị âm thanh</PopoverTitle>
            </PopoverHeader>
            {loadingDevices ? (
              <p className="text-xs text-muted-foreground">Đang tải thiết bị…</p>
            ) : (
              <div className="flex flex-col gap-3">
                <div className="flex flex-col gap-1">
                  <span className="flex items-center gap-1.5 text-[11px] font-medium text-muted-foreground">
                    <Mic className="h-3 w-3" aria-hidden="true" />
                    Micro
                  </span>
                  <Select value={activeMicValue} onValueChange={(val) => void handleMicChange(val)}>
                    <SelectTrigger size="sm" className="w-full min-w-0" aria-label="Chọn micro">
                      <SelectValue placeholder="Micro mặc định">
                        {selectedMicDev?.name ?? "Micro mặc định"}
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent
                      align="start"
                      className="w-max max-w-[min(36rem,calc(100vw-2rem))] min-w-(--anchor-width)"
                    >
                      {devices.inputs.map((dev) => (
                        <SelectItem key={dev.id} value={dev.id} title={dev.name}>
                          {dev.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <div className="mt-1 flex flex-col gap-1.5">
                    <div className="flex items-center justify-between gap-2">
                      <Label
                        htmlFor="call-auto-mic"
                        className="text-[11px] font-normal text-muted-foreground"
                      >
                        Tự động (AGC)
                      </Label>
                      <Switch
                        id="call-auto-mic"
                        size="sm"
                        checked={autoMic}
                        onCheckedChange={(checked) =>
                          void handleAudioConfigChange(
                            checked,
                            autoSpeaker,
                            micVolume,
                            speakerVolume,
                          )
                        }
                      />
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-[11px] text-muted-foreground">
                        {autoMic ? "AGC" : "Thủ công"}
                      </span>
                      <Slider
                        min={0}
                        max={150}
                        step={5}
                        disabled={autoMic}
                        value={autoMic ? micGainPct : micVolume}
                        aria-label="Âm lượng micro"
                        className="flex-1"
                        onChange={(e) =>
                          void handleAudioConfigChange(
                            autoMic,
                            autoSpeaker,
                            Number(e.target.value),
                            speakerVolume,
                          )
                        }
                      />
                      <span className="w-9 text-right font-mono text-[11px] text-muted-foreground tabular-nums">
                        {autoMic ? micGainPct : micVolume}%
                      </span>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-[10px] text-muted-foreground">Mức thu</span>
                      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                        <div
                          className="h-full rounded-full bg-emerald-500 transition-[width] duration-75"
                          style={{ width: `${micLevel}%` }}
                        />
                      </div>
                      <span className="w-9 text-right font-mono text-[10px] text-muted-foreground tabular-nums">
                        {micLevel}%
                      </span>
                    </div>
                  </div>
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
                    <SelectTrigger size="sm" className="w-full min-w-0" aria-label="Chọn loa">
                      <SelectValue placeholder="Loa mặc định">
                        {selectedSpeakerDev?.name ?? "Loa mặc định"}
                      </SelectValue>
                    </SelectTrigger>
                    <SelectContent
                      align="start"
                      className="w-max max-w-[min(36rem,calc(100vw-2rem))] min-w-(--anchor-width)"
                    >
                      {devices.outputs.map((dev) => (
                        <SelectItem key={dev.id} value={dev.id} title={dev.name}>
                          {dev.name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <div className="mt-1 flex flex-col gap-1.5">
                    <div className="flex items-center justify-between gap-2">
                      <Label
                        htmlFor="call-auto-spk"
                        className="text-[11px] font-normal text-muted-foreground"
                      >
                        Tự động (AGC)
                      </Label>
                      <Switch
                        id="call-auto-spk"
                        size="sm"
                        checked={autoSpeaker}
                        onCheckedChange={(checked) =>
                          void handleAudioConfigChange(autoMic, checked, micVolume, speakerVolume)
                        }
                      />
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-[11px] text-muted-foreground">
                        {autoSpeaker ? "AGC" : "Thủ công"}
                      </span>
                      <Slider
                        min={0}
                        max={150}
                        step={5}
                        disabled={autoSpeaker}
                        value={autoSpeaker ? speakerGainPct : speakerVolume}
                        aria-label="Âm lượng loa"
                        className="flex-1"
                        onChange={(e) =>
                          void handleAudioConfigChange(
                            autoMic,
                            autoSpeaker,
                            micVolume,
                            Number(e.target.value),
                          )
                        }
                      />
                      <span className="w-9 text-right font-mono text-[11px] text-muted-foreground tabular-nums">
                        {autoSpeaker ? speakerGainPct : speakerVolume}%
                      </span>
                    </div>
                    <div className="flex items-center gap-2">
                      <span className="text-[10px] text-muted-foreground">Mức phát</span>
                      <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                        <div
                          className="h-full rounded-full bg-emerald-500 transition-[width] duration-75"
                          style={{ width: `${speakerLevel}%` }}
                        />
                      </div>
                      <span className="w-9 text-right font-mono text-[10px] text-muted-foreground tabular-nums">
                        {speakerLevel}%
                      </span>
                    </div>
                  </div>
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

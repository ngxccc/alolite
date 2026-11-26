import { useEffect, useState } from "react";
import { Mic, Volume2 } from "lucide-react";
import { startIncomingRingtone } from "@/features/call/components/incoming-call-modal";
import { ipc, isTauriRuntime } from "@/shared/api/ipc";
import type { AudioDeviceList } from "@/shared/types/ipc.generated";
import { Button } from "@/shared/ui/button";
import { Label } from "@/shared/ui/label";
import { Slider } from "@/shared/ui/slider";
import { Switch } from "@/shared/ui/switch";
import { useCallStore, useUIStore } from "@/stores";
import { SectionHeading, SelectRow, SettingCard, SwitchRow } from "../primitives";
import type { SettingsTabProps } from "../types";

export function CallTab({ settings, updateSettings }: SettingsTabProps) {
  const showToast = useUIStore((s) => s.showToast);
  const phase = useCallStore((s) => s.phase);
  const micLevel = useCallStore((s) => s.micLevel);
  const micGainPct = useCallStore((s) => s.micGainPct);
  const setSelectedDevices = useCallStore((s) => s.setSelectedDevices);
  const setAudioConfig = useCallStore((s) => s.setAudioConfig);

  const [devices, setDevices] = useState<AudioDeviceList>({ inputs: [], outputs: [] });
  const [testingMic, setTestingMic] = useState(false);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    void ipc
      .listAudioDevices()
      .then((list) => setDevices(list))
      .catch(() => undefined);
    return () => {
      void ipc.stopCallMicTest().catch(() => undefined);
    };
  }, []);

  const micOptions = [
    { value: "", label: "Mặc định hệ thống" },
    ...devices.inputs.map((d) => ({ value: d.id, label: d.name })),
  ];
  const spkOptions = [
    { value: "", label: "Mặc định hệ thống" },
    ...devices.outputs.map((d) => ({ value: d.id, label: d.name })),
  ];

  const handleMicSelect = (val: string) => {
    const nextMic = val || null;
    const nextSpk = settings.selected_speaker_device || null;
    updateSettings({ selected_mic_device: val });
    setSelectedDevices(nextMic, nextSpk);
    if (testingMic && isTauriRuntime()) {
      void ipc
        .startCallMicTest(nextMic, settings.auto_mic_agc, settings.mic_gain_percent)
        .catch(() => undefined);
    }
  };

  const handleSpeakerSelect = (val: string) => {
    const nextMic = settings.selected_mic_device || null;
    const nextSpk = val || null;
    updateSettings({ selected_speaker_device: val });
    setSelectedDevices(nextMic, nextSpk);
  };

  const handleToggleMicTest = async () => {
    if (!isTauriRuntime()) return;
    if (testingMic) {
      setTestingMic(false);
      await ipc.stopCallMicTest().catch(() => undefined);
      return;
    }
    try {
      await ipc.startCallMicTest(
        settings.selected_mic_device || null,
        settings.auto_mic_agc,
        settings.mic_gain_percent,
      );
      setTestingMic(true);
    } catch {
      showToast("Không thể thử micro khi đang có cuộc gọi", "error");
    }
  };

  const handleMicAgcChange = (checked: boolean) => {
    updateSettings({ auto_mic_agc: checked });
    setAudioConfig(
      checked,
      settings.auto_speaker_agc,
      settings.mic_gain_percent,
      settings.speaker_gain_percent,
    );
  };

  const handleMicGainChange = (val: number) => {
    updateSettings({ mic_gain_percent: val });
    setAudioConfig(
      settings.auto_mic_agc,
      settings.auto_speaker_agc,
      val,
      settings.speaker_gain_percent,
    );
  };

  const handleSpeakerAgcChange = (checked: boolean) => {
    updateSettings({ auto_speaker_agc: checked });
    setAudioConfig(
      settings.auto_mic_agc,
      checked,
      settings.mic_gain_percent,
      settings.speaker_gain_percent,
    );
  };

  const handleSpeakerGainChange = (val: number) => {
    updateSettings({ speaker_gain_percent: val });
    setAudioConfig(
      settings.auto_mic_agc,
      settings.auto_speaker_agc,
      settings.mic_gain_percent,
      val,
    );
  };

  const displayedMicPct =
    testingMic && settings.auto_mic_agc ? micGainPct : settings.mic_gain_percent;

  return (
    <>
      <SectionHeading>Thiết bị âm thanh cuộc gọi</SectionHeading>
      <SettingCard>
        <SelectRow
          label="Micro đầu vào"
          value={settings.selected_mic_device}
          onChange={handleMicSelect}
          options={micOptions}
        />
        <div className="border-t border-border" />
        <SelectRow
          label="Loa / tai nghe"
          value={settings.selected_speaker_device}
          onChange={handleSpeakerSelect}
          options={spkOptions}
        />
        <div className="border-t border-border" />
        <div className="flex flex-col gap-2.5 px-4 py-3">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2 text-sm text-foreground">
              <Mic className="h-4 w-4 text-muted-foreground" aria-hidden="true" />
              <span>Kiểm tra micro</span>
            </div>
            <Button
              type="button"
              variant={testingMic ? "secondary" : "outline"}
              size="sm"
              disabled={phase !== "idle"}
              onClick={() => void handleToggleMicTest()}
            >
              {testingMic ? "Dừng thử micro" : "Thử micro"}
            </Button>
          </div>
          <div className="flex items-center gap-2">
            <span className="w-16 shrink-0 text-xs text-muted-foreground">Mức thu</span>
            <div className="h-2 flex-1 overflow-hidden rounded-full bg-muted">
              <div
                className="h-full rounded-full bg-online transition-[width] duration-100"
                style={{ width: `${testingMic ? micLevel : 0}%` }}
              />
            </div>
            <span className="w-9 text-right text-xs text-muted-foreground tabular-nums">
              {testingMic ? micLevel : 0}%
            </span>
          </div>
        </div>
      </SettingCard>

      <SectionHeading>Âm lượng & tự động cân bằng (AGC)</SectionHeading>
      <SettingCard>
        <div className="space-y-2 px-4 py-3">
          <div className="flex items-center justify-between">
            <Label htmlFor="settings-mic-agc" className="text-sm font-normal text-foreground">
              Micro · {displayedMicPct}%
            </Label>
            <div className="flex items-center gap-2">
              <span className="text-xs text-muted-foreground">Tự động (AGC)</span>
              <Switch
                id="settings-mic-agc"
                checked={settings.auto_mic_agc}
                onCheckedChange={handleMicAgcChange}
              />
            </div>
          </div>
          <Slider
            min={0}
            max={150}
            step={5}
            disabled={settings.auto_mic_agc}
            value={displayedMicPct}
            onChange={(e) => handleMicGainChange(Number(e.target.value))}
          />
        </div>
        <div className="border-t border-border" />
        <div className="space-y-2 px-4 py-3">
          <div className="flex items-center justify-between">
            <Label htmlFor="settings-spk-agc" className="text-sm font-normal text-foreground">
              Loa · {settings.speaker_gain_percent}%
            </Label>
            <div className="flex items-center gap-2">
              <span className="text-xs text-muted-foreground">Tự động (AGC)</span>
              <Switch
                id="settings-spk-agc"
                checked={settings.auto_speaker_agc}
                onCheckedChange={handleSpeakerAgcChange}
              />
            </div>
          </div>
          <Slider
            min={0}
            max={150}
            step={5}
            disabled={settings.auto_speaker_agc}
            value={settings.speaker_gain_percent}
            onChange={(e) => handleSpeakerGainChange(Number(e.target.value))}
          />
        </div>
      </SettingCard>

      <SectionHeading>Nhạc chuông cuộc gọi đến</SectionHeading>
      <SettingCard>
        <SwitchRow
          label="Phát nhạc chuông khi có cuộc gọi đến"
          checked={settings.call_ringtone_enabled}
          onChange={(v) => updateSettings({ call_ringtone_enabled: v })}
        />
        <div className="border-t border-border" />
        <div className="space-y-2 px-4 py-3">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2 text-sm text-foreground">
              <Volume2 className="h-4 w-4 text-muted-foreground" aria-hidden="true" />
              <span>Âm lượng chuông · {settings.call_ringtone_volume_percent}%</span>
            </div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={!settings.call_ringtone_enabled}
              onClick={() =>
                void startIncomingRingtone({
                  enabled: true,
                  volumePct: settings.call_ringtone_volume_percent,
                  singleBurst: true,
                })
              }
            >
              Nghe thử
            </Button>
          </div>
          <Slider
            min={0}
            max={100}
            step={5}
            disabled={!settings.call_ringtone_enabled}
            value={settings.call_ringtone_volume_percent}
            onChange={(e) =>
              updateSettings({ call_ringtone_volume_percent: Number(e.target.value) })
            }
          />
        </div>
      </SettingCard>
    </>
  );
}

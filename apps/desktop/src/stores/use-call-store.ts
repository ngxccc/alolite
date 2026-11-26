import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import type { CallPhase } from "@/shared/types/ipc.generated";

export interface CallStateSnapshot {
  phase: CallPhase;
  call_id: number | null;
  peer_id: string | null;
  peer_name: string | null;
  peer_avatar: string | null;
  is_incoming: boolean;
  is_muted: boolean;
  is_held: boolean;
  peer_muted?: boolean;
  started_at_ms: number | null;
  end_reason: string | null;
}

interface CallStoreState {
  phase: CallPhase;
  callId: number | null;
  peerId: string | null;
  peerName: string | null;
  peerAvatar: string | null;
  isIncoming: boolean;
  isMuted: boolean;
  isHeld: boolean;
  peerMuted: boolean;
  startedAtMs: number | null;
  endReason: string | null;
  selectedMicId: string | null;
  selectedSpeakerId: string | null;
  autoMic: boolean;
  autoSpeaker: boolean;
  micVolume: number;
  speakerVolume: number;
  micLevel: number;
  speakerLevel: number;
  micGainPct: number;
  speakerGainPct: number;
  applyCallState: (snap: CallStateSnapshot) => void;
  applyAudioLevels: (levels: {
    mic_level: number;
    speaker_level: number;
    mic_gain_pct: number;
    speaker_gain_pct: number;
  }) => void;
  setSelectedDevices: (micId: string | null, speakerId: string | null) => void;
  setAudioConfig: (
    autoMic: boolean,
    autoSpeaker: boolean,
    micVolume: number,
    speakerVolume: number,
  ) => void;
  resetCall: () => void;
}

export const useCallStore = create<CallStoreState>()(
  immer((set) => ({
    phase: "idle",
    callId: null,
    peerId: null,
    peerName: null,
    peerAvatar: null,
    isIncoming: false,
    isMuted: false,
    isHeld: false,
    peerMuted: false,
    startedAtMs: null,
    endReason: null,
    selectedMicId: null,
    selectedSpeakerId: null,
    autoMic: true,
    autoSpeaker: true,
    micVolume: 100,
    speakerVolume: 100,
    micLevel: 0,
    speakerLevel: 0,
    micGainPct: 100,
    speakerGainPct: 100,

    applyCallState: (snap) =>
      set((state) => {
        state.phase = snap.phase;
        state.callId = snap.call_id;
        state.peerId = snap.peer_id;
        state.peerName = snap.peer_name;
        state.peerAvatar = snap.peer_avatar;
        state.isIncoming = snap.is_incoming;
        state.isMuted = snap.is_muted;
        state.isHeld = snap.is_held;
        state.peerMuted = snap.peer_muted ?? false;
        state.startedAtMs = snap.started_at_ms;
        state.endReason = snap.end_reason;
      }),

    applyAudioLevels: (levels) =>
      set((state) => {
        state.micLevel = levels.mic_level;
        state.speakerLevel = levels.speaker_level;
        state.micGainPct = levels.mic_gain_pct;
        state.speakerGainPct = levels.speaker_gain_pct;
      }),

    setSelectedDevices: (micId, speakerId) =>
      set((state) => {
        state.selectedMicId = micId;
        state.selectedSpeakerId = speakerId;
      }),

    setAudioConfig: (autoMic, autoSpeaker, micVolume, speakerVolume) =>
      set((state) => {
        state.autoMic = autoMic;
        state.autoSpeaker = autoSpeaker;
        state.micVolume = micVolume;
        state.speakerVolume = speakerVolume;
      }),

    resetCall: () =>
      set((state) => {
        state.phase = "idle";
        state.callId = null;
        state.peerId = null;
        state.peerName = null;
        state.peerAvatar = null;
        state.isIncoming = false;
        state.isMuted = false;
        state.isHeld = false;
        state.peerMuted = false;
        state.startedAtMs = null;
        state.endReason = null;
        state.micLevel = 0;
        state.speakerLevel = 0;
      }),
  })),
);

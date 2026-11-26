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
  startedAtMs: number | null;
  endReason: string | null;
  selectedMicId: string | null;
  selectedSpeakerId: string | null;
  applyCallState: (snap: CallStateSnapshot) => void;
  setSelectedDevices: (micId: string | null, speakerId: string | null) => void;
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
    startedAtMs: null,
    endReason: null,
    selectedMicId: null,
    selectedSpeakerId: null,

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
        state.startedAtMs = snap.started_at_ms;
        state.endReason = snap.end_reason;
      }),

    setSelectedDevices: (micId, speakerId) =>
      set((state) => {
        state.selectedMicId = micId;
        state.selectedSpeakerId = speakerId;
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
        state.startedAtMs = null;
        state.endReason = null;
      }),
  })),
);

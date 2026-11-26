import { describe, expect, test } from "bun:test";
import { formatCallDuration } from "@/shared/utils/text";
import { useCallStore } from "@/stores/use-call-store";
import { getCallStatusText } from "./components/active-call-bar";
import { startIncomingRingtone } from "./components/incoming-call-modal";

describe("call store and active call bar helpers", () => {
  test("useCallStore tracks mute, hold, and selected audio devices", () => {
    useCallStore.getState().resetCall();
    useCallStore.getState().applyCallState({
      phase: "active",
      call_id: 101,
      peer_id: "peer-1",
      peer_name: "Alice",
      peer_avatar: null,
      is_incoming: false,
      is_muted: true,
      is_held: true,
      started_at_ms: 1000,
      end_reason: null,
    });

    const state = useCallStore.getState();
    expect(state.phase).toBe("active");
    expect(state.isMuted).toBe(true);
    expect(state.isHeld).toBe(true);

    useCallStore.getState().setSelectedDevices("mic-1", "spk-2");
    expect(useCallStore.getState().selectedMicId).toBe("mic-1");
    expect(useCallStore.getState().selectedSpeakerId).toBe("spk-2");
  });

  test("getCallStatusText shows call status in Vietnamese", () => {
    expect(formatCallDuration(65)).toBe("01:05");
    expect(getCallStatusText("active", 65, false)).toBe("Đang trò chuyện · 01:05");
    expect(getCallStatusText("active", 65, true)).toBe("Mic đang tắt · 01:05");
    expect(getCallStatusText("outgoing_ringing", 0, false)).toBe("Đang đổ chuông…");
    expect(getCallStatusText("connecting", 0, false)).toBe("Đang kết nối…");
    expect(getCallStatusText("ending", 0, false)).toBe("Đang kết thúc…");
  });

  test("formatCallDuration clamps negative values and startIncomingRingtone is safe in headless env", () => {
    expect(formatCallDuration(-5)).toBe("00:00");
    expect(formatCallDuration(125.9)).toBe("02:05");
    const stop = startIncomingRingtone();
    expect(() => stop()).not.toThrow();
    const stopMuted = startIncomingRingtone({ enabled: false, volumePct: 80 });
    expect(() => stopMuted()).not.toThrow();
  });

  test("useCallStore tracks peerMuted from CallStateChanged snapshots", () => {
    useCallStore.getState().resetCall();
    useCallStore.getState().applyCallState({
      phase: "active",
      call_id: 901,
      peer_id: "u1",
      peer_name: "Alice",
      peer_avatar: null,
      is_incoming: false,
      is_muted: false,
      is_held: false,
      peer_muted: true,
      started_at_ms: 1000,
      end_reason: null,
    });
    expect(useCallStore.getState().peerMuted).toBe(true);
    useCallStore.getState().resetCall();
    expect(useCallStore.getState().peerMuted).toBe(false);
  });
});

import { describe, expect, test } from "bun:test";
import { useCallStore } from "@/stores/use-call-store";
import { formatCallDuration, getCallStatusText } from "./components/active-call-bar";

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

  test("getCallStatusText shows hold status in Vietnamese when held", () => {
    expect(formatCallDuration(65)).toBe("01:05");
    expect(getCallStatusText("active", 65, false)).toBe("01:05");
    expect(getCallStatusText("active", 65, true)).toBe("Đang tạm giữ");
    expect(getCallStatusText("outgoing_ringing", 0, false)).toBe("Đang đổ chuông…");
    expect(getCallStatusText("connecting", 0, false)).toBe("Đang kết nối…");
    expect(getCallStatusText("ending", 0, false)).toBe("Đang kết thúc…");
  });
});

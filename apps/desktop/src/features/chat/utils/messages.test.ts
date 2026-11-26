import { describe, expect, it } from "bun:test";
import type { ChatMessage } from "@/shared/types/ipc.generated";
import { isFileMessage, isPhotoMessage, parseCallCard, stickerThumbnailUrl } from "./messages";

function msg(patch: Partial<ChatMessage>): ChatMessage {
  return {
    id: "m1",
    sender: "A",
    text: "",
    time: "10:00",
    is_outgoing: false,
    ts: 1000,
    ...patch,
  };
}

describe("chat message utils", () => {
  it("parses voice and missed video call cards", () => {
    const voice = parseCallCard(msg({ text: "[Cuộc gọi thoại 02:15]" }));
    expect(voice?.isVideo).toBe(false);
    expect(voice?.isMissedOrFailed).toBe(false);

    const missed = parseCallCard(msg({ text: "[Cuộc gọi video nhỡ]" }));
    expect(missed?.isVideo).toBe(true);
    expect(missed?.isMissedOrFailed).toBe(true);
  });

  it("parses MessageKind::Call cards with outcome and duration metadata", () => {
    const completed = parseCallCard(
      msg({
        kind: "call",
        is_outgoing: true,
        text: "Cuộc gọi thoại (01:23)",
        call_outcome: "completed",
        call_duration_secs: 83,
      }),
    );
    expect(completed).toEqual({
      title: "Cuộc gọi thoại",
      subtitle: "Cuộc gọi đi · 01:23",
      isMissedOrFailed: false,
      isVideo: false,
      outcome: "completed",
      isOutgoing: true,
    });

    const missed = parseCallCard(
      msg({
        kind: "call",
        is_outgoing: false,
        text: "Cuộc gọi nhỡ",
        call_outcome: "missed",
      }),
    );
    expect(missed).toEqual({
      title: "Cuộc gọi nhỡ",
      subtitle: "Cuộc gọi đến bị nhỡ",
      isMissedOrFailed: true,
      isVideo: false,
      outcome: "missed",
      isOutgoing: false,
    });

    const rejected = parseCallCard(
      msg({
        kind: "call",
        is_outgoing: false,
        text: "Đã từ chối cuộc gọi",
        call_outcome: "rejected",
      }),
    );
    expect(rejected?.isMissedOrFailed).toBe(true);
    expect(rejected?.outcome).toBe("rejected");

    const cancelled = parseCallCard(
      msg({
        kind: "call",
        is_outgoing: true,
        text: "Đã hủy cuộc gọi",
        call_outcome: "cancelled",
      }),
    );
    expect(cancelled?.isMissedOrFailed).toBe(true);
    expect(cancelled?.outcome).toBe("cancelled");
  });

  it("classifies photo vs sticker vs file attachments cleanly", () => {
    expect(isPhotoMessage(msg({ text: "[Hình ảnh]", sticker_id: 123 }))).toBe(false);
    expect(isPhotoMessage(msg({ text: "[Hình ảnh]" }))).toBe(true);
    expect(
      isFileMessage(
        msg({
          link_url: "https://cdn.example/a.pdf",
          link_src: "Tập tin Zalo",
        }),
      ),
    ).toBe(true);
  });

  it("builds Onion and standard sticker CDN URLs", () => {
    expect(stickerThumbnailUrl({ sticker_id: 5, cate_id: 0 })).toContain("/default/thumb/5.png");
    expect(stickerThumbnailUrl({ sticker_id: 98930, cate_id: 61900 })).toContain("eid=98930");
  });
});

import type { ChatMessage, Thread } from "@/shared/types/ipc.generated";
import { useAuthStore, useChatStore } from "@/stores";

export interface ResolvedSender {
  id: string;
  name: string;
  avatar?: string | null;
}

export function isNumericUid(val: string): boolean {
  const trimmed = val.trim();
  return trimmed.length >= 6 && /^\d+$/.test(trimmed);
}

export function useSenderLookup() {
  const uin = useAuthStore((s) => s.uin);
  const displayName = useAuthStore((s) => s.displayName);
  const avatar = useAuthStore((s) => s.avatar);
  const threads = useChatStore((s) => s.threads);
  const byId = new Map<string, { id: string; name: string; avatar?: string | null }>();
  const byName = new Map<string, { id: string; name: string; avatar?: string | null }>();

  if (uin) {
    const me = {
      id: uin,
      name: displayName || "Tôi",
      avatar,
    };
    byId.set(uin, me);
    if (displayName) byName.set(displayName, me);
  }

  for (const t of threads) {
    if (!t.is_group) {
      const entry = { id: t.id, name: t.name, avatar: t.avatar };
      byId.set(t.id, entry);
      if (t.name) byName.set(t.name, entry);
    }
    for (const mem of t.presentation.members) {
      const entry = { id: mem.id, name: mem.name, avatar: mem.avatar };
      byId.set(mem.id, entry);
      if (mem.name && !isNumericUid(mem.name)) {
        byName.set(mem.name, entry);
      }
    }
  }

  const senderLookup = { byId, byName };

  const resolveSender = (msg: ChatMessage, currentThread: Thread): ResolvedSender => {
    if (!currentThread.is_group) {
      return {
        id: currentThread.id,
        name: currentThread.name || msg.sender,
        avatar: currentThread.avatar,
      };
    }
    const rawId = msg.sender_id ?? (isNumericUid(msg.sender) ? msg.sender.trim() : "");
    if (rawId) {
      const foundById = senderLookup.byId.get(rawId);
      if (foundById) {
        return {
          id: foundById.id,
          name: !isNumericUid(foundById.name) ? foundById.name : `Thành viên · ${rawId.slice(-4)}`,
          avatar: foundById.avatar,
        };
      }
    }
    const foundByName = senderLookup.byName.get(msg.sender);
    if (foundByName) {
      return foundByName;
    }
    if (isNumericUid(msg.sender)) {
      return {
        id: msg.sender,
        name: `Thành viên · ${msg.sender.slice(-4)}`,
        avatar: null,
      };
    }
    return {
      id: msg.sender_id ?? msg.sender,
      name: msg.sender || "Thành viên",
      avatar: null,
    };
  };

  const resolveReplySenderName = (raw?: string | null): string => {
    if (!raw) return "Trích dẫn";
    if (isNumericUid(raw)) {
      const found = senderLookup.byId.get(raw.trim());
      if (found && !isNumericUid(found.name)) return found.name;
      return `Thành viên · ${raw.trim().slice(-4)}`;
    }
    return raw;
  };

  return {
    resolveSender,
    resolveReplySenderName,
  };
}

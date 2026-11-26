import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import {
  advanceUnreadSeenFromMessages,
  mergeThreadInto,
  reconcileSentMessage,
  sortThreads,
} from "@/features/threads/utils/threads";
import type {
  AppSettings,
  ChatMessage,
  CoreEventDto,
  DeliveryState,
  LinkMetadata,
  PinnedPackMeta,
  StickerItem,
  Thread,
} from "@/shared/types/ipc.generated";

type MuteEventItem = Extract<
  CoreEventDto,
  { type: "MuteEventsReceived" }
>["payload"]["events"][number];

export interface ChatStoreState {
  threads: Thread[];
  selectedThreadId: string | null;
  threadFilter: "all" | "direct" | "groups";
  searchQuery: string;
  unreadCounts: Record<string, number | undefined>;
  historyReachedStart: Record<string, boolean | undefined>;
  renderLimitByThread: Record<string, number | undefined>;

  replyingTo: ChatMessage | null;
  forwardMsg: ChatMessage | null;
  filterStarredOnly: boolean;
  quoteHighlightId: string | null;
  infoDrawerOpen: boolean;

  sendingMsgIds: Record<string, boolean | undefined>;
  pinnedPacks: PinnedPackMeta[];
  composerStickerQuery: string | null;
  composerStickerSuggestions: StickerItem[];
  linkPreviews: Record<string, LinkMetadata | undefined>;

  // Sync state
  isSyncing: boolean;
  syncProgress: { current: number; total: number } | null;
  lastSyncStats: string | null;
  phoneSyncRunning: boolean;
  phoneSyncProgress: { current: number; total: number; stage?: string } | null;

  // App settings cache
  settings: AppSettings | null;

  // Actions
  setThreads: (threads: Thread[]) => void;
  selectThread: (threadId: string | null) => void;
  setThreadFilter: (filter: "all" | "direct" | "groups") => void;
  setSearchQuery: (query: string) => void;

  setReplyingTo: (msg: ChatMessage | null) => void;
  setForwardMsg: (msg: ChatMessage | null) => void;
  toggleFilterStarredOnly: () => void;
  setQuoteHighlightId: (id: string | null) => void;
  setInfoDrawerOpen: (open: boolean) => void;
  toggleInfoDrawer: () => void;

  appendMessage: (threadId: string, msg: ChatMessage) => void;
  updateMessageDelivery: (cliMsgId: string, delivery: DeliveryState, serverMsgId?: string) => void;
  setSending: (cliMsgId: string, sending: boolean) => void;
  markThreadRead: (threadId: string) => void;
  setRenderLimit: (threadId: string, limit: number) => void;
  expandRenderLimit: (threadId: string, step?: number) => void;
  setHistoryReachedStart: (threadId: string, reached: boolean) => void;
  toggleLocalStar: (threadId: string, msgId: string) => void;

  mergeThreadsUpdate: (updates: Thread[]) => void;
  reconcileSentMessageInThread: (threadId: string, msg: ChatMessage) => void;
  reactToMessage: (
    threadId: string,
    msgId: string | null | undefined,
    ts: number,
    reaction: string,
    revert?: boolean,
  ) => void;
  deleteMessageLocal: (threadId: string, msgId: string | null | undefined, ts: number) => void;
  recallMessageLocal: (threadId: string, msgId: string | null | undefined, ts: number) => void;
  clearAllMessagesLocal: () => void;
  syncMuteSnapshot: (mutedThreads: Record<string, number>) => void;
  applyMuteEvents: (events: MuteEventItem[]) => void;
  resetChatSession: () => void;

  setPinnedPacks: (packs: PinnedPackMeta[]) => void;
  setComposerStickerSuggestions: (items: StickerItem[], query: string | null) => void;
  setLinkPreview: (url: string, meta: LinkMetadata) => void;

  setIsSyncing: (isSyncing: boolean) => void;
  setSyncProgress: (progress: { current: number; total: number } | null) => void;
  setLastSyncStats: (stats: string | null) => void;
  setPhoneSyncRunning: (running: boolean) => void;
  setPhoneSyncProgress: (
    progress: { current: number; total: number; stage?: string } | null,
  ) => void;

  setSettings: (settings: AppSettings | null) => void;
  updateSettings: (partial: Partial<AppSettings>) => void;
}

export const useChatStore = create<ChatStoreState>()(
  immer((set) => ({
    threads: [],
    selectedThreadId: null,
    threadFilter: "all",
    searchQuery: "",
    unreadCounts: {},
    historyReachedStart: {},
    renderLimitByThread: {},

    replyingTo: null,
    forwardMsg: null,
    filterStarredOnly: false,
    quoteHighlightId: null,
    infoDrawerOpen: false,

    sendingMsgIds: {},
    pinnedPacks: [],
    composerStickerQuery: null,
    composerStickerSuggestions: [],
    linkPreviews: {},

    isSyncing: false,
    syncProgress: null,
    lastSyncStats: null,
    phoneSyncRunning: false,
    phoneSyncProgress: null,

    settings: null,

    setThreads: (threads) =>
      set((state) => {
        state.threads = threads;
      }),

    selectThread: (threadId) =>
      set((state) => {
        state.selectedThreadId = threadId;
        state.replyingTo = null;
        state.quoteHighlightId = null;
        if (threadId) {
          state.unreadCounts[threadId] = 0;
          const target = state.threads.find((t) => t.id === threadId);
          if (target) {
            advanceUnreadSeenFromMessages(target);
          }
        }
      }),

    setThreadFilter: (filter) =>
      set((state) => {
        state.threadFilter = filter;
      }),

    setSearchQuery: (query) =>
      set((state) => {
        state.searchQuery = query;
      }),

    setReplyingTo: (msg) =>
      set((state) => {
        state.replyingTo = msg;
      }),

    setForwardMsg: (msg) =>
      set((state) => {
        state.forwardMsg = msg;
      }),

    toggleFilterStarredOnly: () =>
      set((state) => {
        state.filterStarredOnly = !state.filterStarredOnly;
      }),

    setQuoteHighlightId: (id) =>
      set((state) => {
        state.quoteHighlightId = id;
      }),

    setInfoDrawerOpen: (open) =>
      set((state) => {
        state.infoDrawerOpen = open;
      }),

    toggleInfoDrawer: () =>
      set((state) => {
        state.infoDrawerOpen = !state.infoDrawerOpen;
      }),

    appendMessage: (threadId, msg) =>
      set((state) => {
        const thread = state.threads.find((t) => t.id === threadId);
        if (thread) {
          // Avoid duplicate messages
          const existingIdx = thread.messages.findIndex(
            (m) =>
              (m.id && m.id === msg.id) ||
              (m.client_message_id &&
                msg.client_message_id &&
                m.client_message_id === msg.client_message_id),
          );
          if (existingIdx !== -1) {
            const existing = thread.messages[existingIdx];
            if (existing) {
              Object.assign(existing, msg);
            }
          } else {
            thread.messages.push(msg);
          }
          thread.preview = msg.text || thread.preview;
          thread.time = msg.time || thread.time;
        }
        if (state.selectedThreadId !== threadId) {
          state.unreadCounts[threadId] = (state.unreadCounts[threadId] ?? 0) + 1;
        }
      }),

    updateMessageDelivery: (cliMsgId, delivery, serverMsgId) =>
      set((state) => {
        for (const thread of state.threads) {
          const msg = thread.messages.find(
            (m) => m.client_message_id === cliMsgId || (serverMsgId && m.id === serverMsgId),
          );
          if (msg) {
            msg.delivery = delivery;
            if (serverMsgId) msg.id = serverMsgId;
            break;
          }
        }
        delete state.sendingMsgIds[cliMsgId];
      }),

    setSending: (cliMsgId, sending) =>
      set((state) => {
        if (sending) {
          state.sendingMsgIds[cliMsgId] = true;
        } else {
          delete state.sendingMsgIds[cliMsgId];
        }
      }),

    markThreadRead: (threadId) =>
      set((state) => {
        state.unreadCounts[threadId] = 0;
        const target = state.threads.find((t) => t.id === threadId);
        if (target) {
          advanceUnreadSeenFromMessages(target);
        }
      }),

    setRenderLimit: (threadId, limit) =>
      set((state) => {
        state.renderLimitByThread[threadId] = limit;
      }),

    expandRenderLimit: (threadId, step = 150) =>
      set((state) => {
        const cur = state.renderLimitByThread[threadId] ?? 150;
        state.renderLimitByThread[threadId] = cur + step;
      }),

    setHistoryReachedStart: (threadId, reached) =>
      set((state) => {
        state.historyReachedStart[threadId] = reached;
      }),

    toggleLocalStar: (threadId, msgId) =>
      set((state) => {
        const thread = state.threads.find((t) => t.id === threadId);
        if (!thread) return;
        const msg = thread.messages.find((m) => m.id === msgId);
        if (msg) msg.is_starred = !msg.is_starred;
      }),

    mergeThreadsUpdate: (updates) =>
      set((state) => {
        if (updates.length === 0) return;
        const byId = new Map(state.threads.map((t) => [t.id, t]));
        for (const incoming of updates) {
          if (!incoming.id || incoming.id === "0") continue;
          const existing = byId.get(incoming.id);
          if (existing) {
            const merged = mergeThreadInto(existing, incoming);
            if (incoming.id === state.selectedThreadId) {
              merged.unread.unread_ids = [];
              merged.unread.marked_unread = false;
              state.unreadCounts[incoming.id] = 0;
            } else {
              state.unreadCounts[incoming.id] = merged.unread.unread_ids.length;
            }
            byId.set(incoming.id, merged);
          } else {
            if (incoming.id === state.selectedThreadId) {
              incoming.unread.unread_ids = [];
              incoming.unread.mention_ids = [];
              incoming.unread.marked_unread = false;
              state.unreadCounts[incoming.id] = 0;
            } else {
              state.unreadCounts[incoming.id] = incoming.unread.unread_ids.length;
            }
            byId.set(incoming.id, incoming);
          }
        }
        const rawList = Array.from(byId.values());
        const sorted = state.settings ? sortThreads(rawList, state.settings) : rawList;
        state.threads = sorted;
        state.selectedThreadId =
          state.selectedThreadId && sorted.some((t) => t.id === state.selectedThreadId)
            ? state.selectedThreadId
            : (sorted[0]?.id ?? null);
      }),

    reconcileSentMessageInThread: (threadId, msg) =>
      set((state) => {
        const idx = state.threads.findIndex((t) => t.id === threadId);
        if (idx === -1) return;
        const existing = state.threads[idx];
        if (existing) {
          state.threads[idx] = reconcileSentMessage(existing, msg);
        }
      }),

    reactToMessage: (threadId, msgId, ts, reaction, revert = false) =>
      set((state) => {
        const thread = state.threads.find((t) => t.id === threadId);
        if (!thread) return;
        for (const m of thread.messages) {
          const matches = (msgId && m.id === msgId) || (!msgId && m.ts === ts);
          if (!matches) continue;
          const current = m.reactions ?? [];
          if (revert) {
            m.reactions = current.filter((r) => r !== reaction);
          } else if (!current.includes(reaction)) {
            m.reactions = [...current, reaction];
          }
        }
      }),

    deleteMessageLocal: (threadId, msgId, ts) =>
      set((state) => {
        const thread = state.threads.find((t) => t.id === threadId);
        if (!thread) return;
        thread.messages = thread.messages.filter(
          (m) => !((msgId && m.id === msgId) || (!msgId && m.ts === ts)),
        );
      }),

    recallMessageLocal: (threadId, msgId, ts) =>
      set((state) => {
        const thread = state.threads.find((t) => t.id === threadId);
        if (!thread) return;
        for (const m of thread.messages) {
          const matches = (msgId && m.id === msgId) || (!msgId && m.ts === ts);
          if (!matches) continue;
          m.text = "[Tin nhắn đã thu hồi]";
          m.sticker_id = undefined;
          m.link_url = undefined;
          m.link_thumb = undefined;
        }
      }),

    clearAllMessagesLocal: () =>
      set((state) => {
        for (const t of state.threads) {
          t.messages = [];
        }
      }),

    syncMuteSnapshot: (mutedThreads) =>
      set((state) => {
        if (!state.settings) return;
        state.settings.muted_threads = {
          ...state.settings.muted_threads,
          ...mutedThreads,
        };
      }),

    applyMuteEvents: (events) =>
      set((state) => {
        if (!state.settings) return;
        for (const ev of events) {
          if (ev.muted) {
            if (ev.settings_until != null) {
              state.settings.muted_threads[ev.thread_id] = ev.settings_until;
            }
          } else {
            delete state.settings.muted_threads[ev.thread_id];
          }
          const thread = state.threads.find((t) => t.id === ev.thread_id);
          if (thread) {
            thread.presentation.muted = ev.muted;
            thread.unread.muted = ev.muted;
            thread.unread.muted_until = ev.muted_until;
          }
        }
      }),

    resetChatSession: () =>
      set((state) => {
        state.threads = [];
        state.selectedThreadId = null;
        state.unreadCounts = {};
        state.historyReachedStart = {};
        state.renderLimitByThread = {};
        state.replyingTo = null;
        state.forwardMsg = null;
        state.quoteHighlightId = null;
      }),

    setPinnedPacks: (packs) =>
      set((state) => {
        state.pinnedPacks = packs;
      }),

    setComposerStickerSuggestions: (items, query) =>
      set((state) => {
        state.composerStickerSuggestions = items;
        state.composerStickerQuery = query;
      }),

    setLinkPreview: (url, meta) =>
      set((state) => {
        state.linkPreviews[url] = meta;
      }),

    setIsSyncing: (isSyncing) =>
      set((state) => {
        state.isSyncing = isSyncing;
      }),

    setSyncProgress: (progress) =>
      set((state) => {
        state.syncProgress = progress;
      }),

    setLastSyncStats: (stats) =>
      set((state) => {
        state.lastSyncStats = stats;
      }),

    setPhoneSyncRunning: (running) =>
      set((state) => {
        state.phoneSyncRunning = running;
      }),

    setPhoneSyncProgress: (progress) =>
      set((state) => {
        state.phoneSyncProgress = progress;
      }),

    setSettings: (settings) =>
      set((state) => {
        state.settings = settings;
        if (settings) {
          state.threads = sortThreads(state.threads, settings);
        }
      }),

    updateSettings: (partial) =>
      set((state) => {
        if (state.settings) {
          Object.assign(state.settings, partial);
        }
      }),
  })),
);

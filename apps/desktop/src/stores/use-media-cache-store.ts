import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import type { LinkMetadata, StickerItem, StickerPackInfo } from "@/shared/types/ipc.generated";

export const DEFAULT_MEDIA_CACHE_LIMIT = 250;

export interface MediaCacheState {
  mediaCache: Record<string, string | undefined>;
  mediaOrder: string[];
  maxEntries: number;
  popularStickers: StickerItem[];
  onionStickers: StickerItem[];
  stickerStorePacks: StickerPackInfo[];
  stickerStoreLoading: boolean;
  stickerPackItems: Record<number, StickerItem[] | undefined>;
  stickerSearchResults: StickerItem[];
  stickerSearchLoading: boolean;
  stickerSuggestKeywords: string[];
  composerStickerSuggestions: StickerItem[];
  linkMetadataCache: Record<string, LinkMetadata | undefined>;

  putMedia: (key: string, dataUrl: string) => void;
  setMaxEntries: (limit: number) => void;
  setPopularStickers: (items: StickerItem[]) => void;
  setOnionStickers: (items: StickerItem[]) => void;
  setStickerStorePacks: (packs: StickerPackInfo[]) => void;
  setStickerStoreLoading: (loading: boolean) => void;
  setStickerPackItems: (cateId: number, items: StickerItem[]) => void;
  setStickerSearchResults: (items: StickerItem[]) => void;
  setStickerSearchLoading: (loading: boolean) => void;
  setStickerSuggestKeywords: (keywords: string[]) => void;
  setComposerStickerSuggestions: (items: StickerItem[]) => void;
  setLinkMetadata: (url: string, meta: LinkMetadata) => void;
  resetMediaCache: () => void;
}

export const useMediaCacheStore = create<MediaCacheState>()(
  immer((set) => ({
    mediaCache: {},
    mediaOrder: [],
    maxEntries: DEFAULT_MEDIA_CACHE_LIMIT,
    popularStickers: [],
    onionStickers: [],
    stickerStorePacks: [],
    stickerStoreLoading: false,
    stickerPackItems: {},
    stickerSearchResults: [],
    stickerSearchLoading: false,
    stickerSuggestKeywords: [],
    composerStickerSuggestions: [],
    linkMetadataCache: {},

    putMedia: (key, dataUrl) =>
      set((state) => {
        // Refresh recency when updating an existing key so hot assets stay cached.
        const existingIdx = state.mediaOrder.indexOf(key);
        if (existingIdx !== -1) {
          state.mediaOrder.splice(existingIdx, 1);
        }
        state.mediaCache[key] = dataUrl;
        state.mediaOrder.push(key);

        const limit = Math.max(1, state.maxEntries);
        while (state.mediaOrder.length > limit) {
          const evicted = state.mediaOrder.shift();
          if (evicted !== undefined) {
            delete state.mediaCache[evicted];
          }
        }
      }),

    setMaxEntries: (limit) =>
      set((state) => {
        const clamped = Math.max(1, limit);
        state.maxEntries = clamped;
        while (state.mediaOrder.length > clamped) {
          const evicted = state.mediaOrder.shift();
          if (evicted !== undefined) {
            delete state.mediaCache[evicted];
          }
        }
      }),

    setPopularStickers: (items) =>
      set((state) => {
        state.popularStickers = items;
      }),

    setOnionStickers: (items) =>
      set((state) => {
        state.onionStickers = items;
      }),

    setStickerStorePacks: (packs) =>
      set((state) => {
        state.stickerStorePacks = packs;
        state.stickerStoreLoading = false;
      }),

    setStickerStoreLoading: (loading) =>
      set((state) => {
        state.stickerStoreLoading = loading;
      }),

    setStickerPackItems: (cateId, items) =>
      set((state) => {
        state.stickerPackItems[cateId] = items;
      }),

    setStickerSearchResults: (items) =>
      set((state) => {
        state.stickerSearchResults = items;
        state.stickerSearchLoading = false;
      }),

    setStickerSearchLoading: (loading) =>
      set((state) => {
        state.stickerSearchLoading = loading;
      }),

    setStickerSuggestKeywords: (keywords) =>
      set((state) => {
        state.stickerSuggestKeywords = keywords;
      }),

    setComposerStickerSuggestions: (items) =>
      set((state) => {
        state.composerStickerSuggestions = items;
      }),

    setLinkMetadata: (url, meta) =>
      set((state) => {
        state.linkMetadataCache[url] = meta;
      }),

    resetMediaCache: () =>
      set((state) => {
        state.mediaCache = {};
        state.mediaOrder = [];
        state.stickerStorePacks = [];
        state.stickerStoreLoading = false;
        state.stickerPackItems = {};
        state.stickerSearchResults = [];
        state.stickerSearchLoading = false;
        state.composerStickerSuggestions = [];
        state.linkMetadataCache = {};
      }),
  })),
);

import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import type { DevDiagnosticsSnapshot, UpdateCheckResult } from "@/shared/types/ipc.generated";
import type { ImageViewerState, NavSection, SettingsTab } from "./types";

let toastSeq = 1;

export interface UIState {
  themeMode: "light" | "dark" | "system";
  activeSection: NavSection;
  devMode: boolean;
  appLogoDataUrl: string;
  activeDbPath: string;
  inlineVideoSupported: boolean;
  inlineVideoReason: string | null;
  devDiagnostics: DevDiagnosticsSnapshot | null;

  // Modals & Panels
  settingsOpen: boolean;
  activeSettingsTab: SettingsTab;
  devToolsOpen: boolean;
  phoneSyncOpen: boolean;
  phoneLookupOpen: boolean;
  wipeConfirmOpen: boolean;
  imageViewerState: ImageViewerState | null;

  // In-chat Search
  inChatSearchOpen: boolean;
  inChatSearchQuery: string;

  // App Updates
  updateChecking: boolean;
  updateInfo: UpdateCheckResult | null;
  updateError: string | null;
  updateDownloading: boolean;
  updateDownloadMessage: string;
  updateProgress: { downloaded: number; total: number } | null;

  // Status & Toast
  statusMessage: string;
  toast: { id?: number; message: string; kind?: "info" | "error" | "success" } | null;

  // Actions
  setThemeMode: (mode: "light" | "dark" | "system") => void;
  setActiveSection: (section: NavSection) => void;
  setDevMode: (devMode: boolean) => void;
  setAppMeta: (meta: {
    appLogoDataUrl?: string;
    activeDbPath?: string;
    inlineVideoSupported?: boolean;
    inlineVideoReason?: string | null;
  }) => void;
  setDevDiagnostics: (diagnostics: DevDiagnosticsSnapshot | null) => void;

  setSettingsOpen: (open: boolean, tab?: SettingsTab) => void;
  setActiveSettingsTab: (tab: SettingsTab) => void;
  setDevToolsOpen: (open: boolean) => void;
  setPhoneSyncOpen: (open: boolean) => void;
  setPhoneLookupOpen: (open: boolean) => void;
  setWipeConfirmOpen: (open: boolean) => void;
  setImageViewerState: (viewer: ImageViewerState | null) => void;

  setInChatSearchOpen: (open: boolean) => void;
  setInChatSearchQuery: (query: string) => void;

  setUpdateChecking: (checking: boolean) => void;
  setUpdateCheckFinished: (
    ok: boolean,
    info: UpdateCheckResult | null,
    error: string | null,
    silent: boolean,
  ) => void;
  setUpdateDownloadStatus: (downloading: boolean, message: string) => void;
  setUpdateDownloadProgress: (progress: { downloaded: number; total: number } | null) => void;
  setUpdateApplied: (version: string) => void;

  setStatusMessage: (message: string) => void;
  showToast: (message: string, kind?: "info" | "error" | "success") => void;
  setToast: (
    toast: { id?: number; message: string; kind?: "info" | "error" | "success" } | null,
  ) => void;
  clearToast: () => void;
}

export const useUIStore = create<UIState>()(
  immer((set) => ({
    themeMode: "dark",
    activeSection: "chat",
    devMode: false,
    appLogoDataUrl: "",
    activeDbPath: "",
    inlineVideoSupported: false,
    inlineVideoReason: null,
    devDiagnostics: null,

    settingsOpen: false,
    activeSettingsTab: "general",
    devToolsOpen: false,
    phoneSyncOpen: false,
    phoneLookupOpen: false,
    wipeConfirmOpen: false,
    imageViewerState: null,

    inChatSearchOpen: false,
    inChatSearchQuery: "",

    updateChecking: false,
    updateInfo: null,
    updateError: null,
    updateDownloading: false,
    updateDownloadMessage: "",
    updateProgress: null,

    statusMessage: "Trực tuyến · Sẵn sàng",
    toast: null,

    setThemeMode: (mode) =>
      set((state) => {
        state.themeMode = mode;
      }),

    setActiveSection: (section) =>
      set((state) => {
        state.activeSection = section;
      }),

    setDevMode: (devMode) =>
      set((state) => {
        state.devMode = devMode;
      }),

    setAppMeta: (meta) =>
      set((state) => {
        if (meta.appLogoDataUrl !== undefined) state.appLogoDataUrl = meta.appLogoDataUrl;
        if (meta.activeDbPath !== undefined) state.activeDbPath = meta.activeDbPath;
        if (meta.inlineVideoSupported !== undefined)
          state.inlineVideoSupported = meta.inlineVideoSupported;
        if (meta.inlineVideoReason !== undefined) state.inlineVideoReason = meta.inlineVideoReason;
      }),

    setDevDiagnostics: (diagnostics) =>
      set((state) => {
        state.devDiagnostics = diagnostics;
      }),

    setSettingsOpen: (open, tab) =>
      set((state) => {
        state.settingsOpen = open;
        if (tab) state.activeSettingsTab = tab;
      }),

    setActiveSettingsTab: (tab) =>
      set((state) => {
        state.activeSettingsTab = tab;
      }),

    setDevToolsOpen: (open) =>
      set((state) => {
        state.devToolsOpen = open;
      }),

    setPhoneSyncOpen: (open) =>
      set((state) => {
        state.phoneSyncOpen = open;
      }),

    setPhoneLookupOpen: (open) =>
      set((state) => {
        state.phoneLookupOpen = open;
      }),

    setWipeConfirmOpen: (open) =>
      set((state) => {
        state.wipeConfirmOpen = open;
      }),

    setImageViewerState: (viewer) =>
      set((state) => {
        state.imageViewerState = viewer;
      }),

    setInChatSearchOpen: (open) =>
      set((state) => {
        state.inChatSearchOpen = open;
        if (!open) state.inChatSearchQuery = "";
      }),

    setInChatSearchQuery: (query) =>
      set((state) => {
        state.inChatSearchQuery = query;
      }),

    setUpdateChecking: (checking) =>
      set((state) => {
        state.updateChecking = checking;
        state.updateError = null;
      }),

    setUpdateCheckFinished: (ok, info, error, silent) =>
      set((state) => {
        state.updateChecking = false;
        if (ok) {
          state.updateInfo = info;
          state.updateError = null;
          if (!silent && info && !info.has_update) {
            toastSeq += 1;
            state.toast = { id: toastSeq, message: "Bạn đang dùng phiên bản mới nhất" };
          }
        } else {
          state.updateError = error ?? "Không kiểm tra được cập nhật";
        }
      }),

    setUpdateDownloadStatus: (downloading, message) =>
      set((state) => {
        state.updateDownloading = downloading;
        state.updateDownloadMessage = message;
      }),

    setUpdateDownloadProgress: (progress) =>
      set((state) => {
        state.updateProgress = progress;
      }),

    setUpdateApplied: (version) =>
      set((state) => {
        state.updateDownloading = false;
        state.updateDownloadMessage = `Đã cập nhật lên v${version} — đang khởi động lại…`;
        toastSeq += 1;
        state.toast = { id: toastSeq, message: `Đã cập nhật v${version}`, kind: "success" };
      }),

    setStatusMessage: (message) =>
      set((state) => {
        state.statusMessage = message;
      }),

    showToast: (message, kind) =>
      set((state) => {
        toastSeq += 1;
        state.toast = { id: toastSeq, message, kind };
      }),

    setToast: (toast) =>
      set((state) => {
        if (!toast) {
          state.toast = null;
          return;
        }
        toastSeq += 1;
        state.toast = { id: toast.id ?? toastSeq, message: toast.message, kind: toast.kind };
      }),

    clearToast: () =>
      set((state) => {
        state.toast = null;
      }),
  })),
);

import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import type { AccountProfile } from "@/shared/types/ipc.generated";

export interface AuthState {
  bootstrapped: boolean;
  appVersion: string;
  loggedIn: boolean;
  uin: string;
  displayName: string;
  avatar: string | null;
  qrDataUrl: string | null;
  qrStatus: string;
  qrScannedName: string | null;
  qrScannedAvatar: string | null;
  loginInProgress: boolean;
  loginError: string;
  captchaToken: string | null;
  captchaImg: string | null;
  savedAccounts: AccountProfile[];

  // Actions
  setBootstrapped: (bootstrapped: boolean, appVersion?: string) => void;
  setLoggedIn: (profile: { uin: string; displayName: string; avatar?: string | null }) => void;
  setLoggedOut: () => void;
  setQrReady: (qrDataUrl: string | null) => void;
  setQrScanned: (name?: string | null, avatar?: string | null) => void;
  setQrExpired: (message?: string) => void;
  setCaptcha: (token: string | null, img: string | null) => void;
  setLoginProgress: (inProgress: boolean, error?: string) => void;
  setSavedAccounts: (accounts: AccountProfile[]) => void;
  deleteSavedAccount: (uin: string) => void;
}

export const useAuthStore = create<AuthState>()(
  immer((set) => ({
    bootstrapped: false,
    appVersion: "0.1.0",
    loggedIn: false,
    uin: "",
    displayName: "",
    avatar: null,
    qrDataUrl: null,
    qrStatus: "Chờ quét mã QR...",
    qrScannedName: null,
    qrScannedAvatar: null,
    loginInProgress: false,
    loginError: "",
    captchaToken: null,
    captchaImg: null,
    savedAccounts: [],

    setBootstrapped: (bootstrapped, appVersion) =>
      set((state) => {
        state.bootstrapped = bootstrapped;
        if (appVersion) state.appVersion = appVersion;
      }),

    setLoggedIn: (profile) =>
      set((state) => {
        state.loggedIn = true;
        state.uin = profile.uin;
        state.displayName = profile.displayName;
        state.avatar = profile.avatar ?? null;
        state.loginInProgress = false;
        state.loginError = "";
        state.captchaToken = null;
        state.captchaImg = null;
        state.qrDataUrl = null;
        state.qrScannedName = null;
        state.qrScannedAvatar = null;
      }),

    setLoggedOut: () =>
      set((state) => {
        state.loggedIn = false;
        state.uin = "";
        state.displayName = "";
        state.avatar = null;
        state.loginInProgress = false;
        state.loginError = "";
        state.captchaToken = null;
        state.captchaImg = null;
        state.qrDataUrl = null;
        state.qrScannedName = null;
        state.qrScannedAvatar = null;
        state.qrStatus = "Đang xin mã QR mới…";
      }),

    setQrReady: (qrDataUrl) =>
      set((state) => {
        state.qrDataUrl = qrDataUrl;
        state.qrStatus = "Mở Zalo trên điện thoại và quét mã QR để đăng nhập";
        state.qrScannedName = null;
        state.qrScannedAvatar = null;
        state.loginInProgress = false;
        state.loginError = "";
      }),

    setQrScanned: (name, avatar) =>
      set((state) => {
        state.qrScannedName = name ?? null;
        state.qrScannedAvatar = avatar ?? null;
        state.qrStatus = name
          ? `Đã quét (${name}) — Vui lòng xác nhận đăng nhập trên điện thoại`
          : "Đã quét mã QR. Vui lòng xác nhận trên điện thoại";
      }),

    setQrExpired: (message) =>
      set((state) => {
        state.qrStatus = message ?? "Mã QR đã hết hạn. Đang tạo mã mới...";
        state.qrDataUrl = null;
        state.qrScannedName = null;
      }),

    setCaptcha: (token, img) =>
      set((state) => {
        state.loginInProgress = false;
        state.loginError = "";
        state.captchaToken = token;
        state.captchaImg = img;
      }),

    setLoginProgress: (inProgress, error = "") =>
      set((state) => {
        state.loginInProgress = inProgress;
        state.loginError = error;
      }),

    setSavedAccounts: (accounts) =>
      set((state) => {
        state.savedAccounts = accounts;
      }),

    deleteSavedAccount: (uin) =>
      set((state) => {
        state.savedAccounts = state.savedAccounts.filter((a) => a.uin !== uin);
      }),
  })),
);

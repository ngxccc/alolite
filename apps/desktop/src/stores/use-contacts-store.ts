import { create } from "zustand";
import { immer } from "zustand/middleware/immer";
import type { FriendshipStatus, PhoneLookupResult } from "@/shared/types/ipc.generated";
import type { ContactsSubTab } from "./types";

export interface ContactsStoreState {
  contactsSubTab: ContactsSubTab;
  contactsSearchQuery: string;
  phoneLookupQuery: string;
  phoneLookupLoading: boolean;
  phoneLookupResult: PhoneLookupResult | null;
  phoneLookupError: string | null;
  friendStatusByUid: Record<string, FriendshipStatus | undefined>;
  userOnlineMap: Record<string, [boolean, string] | undefined>;

  // Actions
  setContactsSubTab: (tab: ContactsSubTab) => void;
  setContactsSearchQuery: (query: string) => void;
  setPhoneLookupQuery: (query: string) => void;
  setPhoneLookupLoading: (loading: boolean) => void;
  setPhoneLookupResult: (result: PhoneLookupResult | null) => void;
  setPhoneLookupError: (error: string | null) => void;
  setFriendStatus: (uid: string, status: FriendshipStatus) => void;
  setUserOnline: (uid: string, online: [boolean, string]) => void;
  resetPhoneLookup: () => void;
  resetContacts: () => void;
}

export const useContactsStore = create<ContactsStoreState>()(
  immer((set) => ({
    contactsSubTab: "friends",
    contactsSearchQuery: "",
    phoneLookupQuery: "",
    phoneLookupLoading: false,
    phoneLookupResult: null,
    phoneLookupError: null,
    friendStatusByUid: {},
    userOnlineMap: {},

    setContactsSubTab: (tab) =>
      set((state) => {
        state.contactsSubTab = tab;
      }),

    setContactsSearchQuery: (query) =>
      set((state) => {
        state.contactsSearchQuery = query;
      }),

    setPhoneLookupQuery: (query) =>
      set((state) => {
        state.phoneLookupQuery = query;
      }),

    setPhoneLookupLoading: (loading) =>
      set((state) => {
        state.phoneLookupLoading = loading;
      }),

    setPhoneLookupResult: (result) =>
      set((state) => {
        state.phoneLookupResult = result;
        state.phoneLookupLoading = false;
        state.phoneLookupError = null;
      }),

    setPhoneLookupError: (error) =>
      set((state) => {
        state.phoneLookupError = error;
        state.phoneLookupLoading = false;
      }),

    setFriendStatus: (uid, status) =>
      set((state) => {
        state.friendStatusByUid[uid] = status;
      }),

    setUserOnline: (uid, online) =>
      set((state) => {
        state.userOnlineMap[uid] = online;
      }),

    resetPhoneLookup: () =>
      set((state) => {
        state.phoneLookupQuery = "";
        state.phoneLookupLoading = false;
        state.phoneLookupResult = null;
        state.phoneLookupError = null;
      }),

    resetContacts: () =>
      set((state) => {
        state.contactsSearchQuery = "";
        state.phoneLookupQuery = "";
        state.phoneLookupLoading = false;
        state.phoneLookupResult = null;
        state.phoneLookupError = null;
        state.friendStatusByUid = {};
        state.userOnlineMap = {};
      }),
  })),
);

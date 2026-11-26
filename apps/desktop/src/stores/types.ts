export type NavSection = "chat" | "contacts";

export type ContactsSubTab = "friends" | "groups" | "friend_requests";

export type SettingsTab =
  | "general"
  | "account"
  | "privacy"
  | "sync"
  | "appearance"
  | "messages"
  | "utilities"
  | "update";

export interface ImageViewerState {
  texKey: string;
  hdTexKey?: string;
  fullUrl: string;
  title: string;
  sender: string;
  time: string;
}

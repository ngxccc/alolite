export type NavSection = "chat" | "contacts";

export type ContactsSubTab = "friends" | "groups" | "friend_requests" | "group_invites";

export type SettingsTab =
  | "general"
  | "account"
  | "privacy"
  | "sync"
  | "appearance"
  | "notifications"
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

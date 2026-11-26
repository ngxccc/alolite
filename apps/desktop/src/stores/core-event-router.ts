import { sortThreads } from "@/features/threads/utils/threads";
import type {
  ChatMessage,
  CoreBootstrapSnapshot,
  CoreEventDto,
} from "@/shared/types/ipc.generated";
import { formatClockNow } from "@/shared/utils/text";
import { useAuthStore } from "./use-auth-store";
import { useCallStore } from "./use-call-store";
import { useChatStore } from "./use-chat-store";
import { useContactsStore } from "./use-contacts-store";
import { useMediaCacheStore } from "./use-media-cache-store";
import { useUIStore } from "./use-ui-store";

/**
 * Hydrate all domain stores from a single `CoreBootstrapSnapshot` round trip.
 */
export function bootstrapStores(snap: CoreBootstrapSnapshot): void {
  const sorted = sortThreads(snap.threads, snap.settings);
  const auth = useAuthStore.getState();
  const ui = useUIStore.getState();
  const chat = useChatStore.getState();
  const media = useMediaCacheStore.getState();

  auth.setBootstrapped(true, snap.app_version);
  if (snap.logged_in) {
    auth.setLoggedIn({
      uin: snap.uin,
      displayName: snap.display_name,
      avatar: snap.avatar,
    });
  } else {
    auth.setLoggedOut();
  }
  auth.setSavedAccounts(snap.saved_accounts);

  ui.setDevMode(snap.dev_mode);
  ui.setAppMeta({
    appLogoDataUrl: snap.app_logo_data_url,
    activeDbPath: snap.active_db_path,
    inlineVideoSupported: snap.inline_video_supported,
    inlineVideoReason: snap.inline_video_reason ?? null,
  });
  const tm = snap.settings.theme_mode;
  ui.setThemeMode(tm === "light" || tm === "system" ? tm : "dark");

  chat.setSettings(snap.settings);
  chat.setThreads(sorted);
  chat.selectThread(sorted[0]?.id ?? null);
  chat.setInfoDrawerOpen(snap.settings.info_drawer_open);
  chat.setPinnedPacks(snap.pinned_packs);
  chat.setIsSyncing(snap.sync_running);
  chat.setPhoneSyncRunning(snap.phone_sync_running);

  media.setPopularStickers(snap.popular_stickers);
  media.setOnionStickers(snap.onion_stickers);
  for (const p of snap.pinned_packs) {
    media.setStickerPackItems(p.cate_id, p.items);
  }
}

/**
 * Route an incoming `CoreEventDto` directly to the owning Zustand domain store(s).
 */
export function dispatchCoreEvent(event: CoreEventDto): void {
  const auth = useAuthStore.getState();
  const ui = useUIStore.getState();
  const chat = useChatStore.getState();
  const contacts = useContactsStore.getState();
  const media = useMediaCacheStore.getState();

  switch (event.type) {
    case "QrReady":
      auth.setQrReady(event.payload.qr_data_url || null);
      return;

    case "QrScanned":
      auth.setQrScanned(event.payload.name, event.payload.avatar);
      return;

    case "QrExpired":
      auth.setQrExpired(event.payload.message);
      return;

    case "LoginSuccess":
      auth.setLoggedIn({
        uin: event.payload.uin,
        displayName: event.payload.name,
        avatar: event.payload.avatar,
      });
      ui.setStatusMessage("Đã đăng nhập");
      ui.showToast(`Chào mừng ${event.payload.name}`, "success");
      return;

    case "LoginFailed":
      auth.setLoginProgress(false, event.payload.message);
      ui.setStatusMessage(event.payload.message);
      return;

    case "CaptchaRequired":
      auth.setCaptcha(event.payload.token, event.payload.captcha_img);
      return;

    case "LoggedOut":
    case "SessionExpired":
      auth.setLoggedOut();
      chat.resetChatSession();
      contacts.resetContacts();
      media.resetMediaCache();
      useCallStore.getState().resetCall();
      ui.setSettingsOpen(false);
      ui.setStatusMessage(event.type === "SessionExpired" ? event.payload.message : "Đã đăng xuất");
      return;

    case "AccountSwitched": {
      const sorted = chat.settings
        ? sortThreads(event.payload.threads, chat.settings)
        : event.payload.threads;
      auth.setLoggedIn({
        uin: event.payload.uin,
        displayName: event.payload.name,
        avatar: event.payload.avatar,
      });
      chat.resetChatSession();
      contacts.resetContacts();
      media.resetMediaCache();
      useCallStore.getState().resetCall();
      chat.setThreads(sorted);
      chat.selectThread(sorted[0]?.id ?? null);
      ui.setSettingsOpen(false);
      ui.setStatusMessage(`Đã chuyển sang tài khoản ${event.payload.name}`);
      ui.showToast(`Đã chuyển sang ${event.payload.name}`, "success");
      return;
    }

    case "SessionUnreachable":
      ui.setStatusMessage(`Mất kết nối tạm thời: ${event.payload.message}`);
      return;

    case "ThreadsUpdated":
      if (!auth.loggedIn || event.payload.threads.length === 0) return;
      chat.mergeThreadsUpdate(event.payload.threads);
      ui.setStatusMessage(`Trực tuyến · Cập nhật ${formatClockNow()}`);
      return;

    case "MessageSent": {
      const now = formatClockNow();
      const finalId = event.payload.msg_id || `out_${event.payload.client_message_id}`;
      const msg: ChatMessage = {
        id: finalId,
        client_message_id: event.payload.client_message_id,
        sender: "Tôi",
        text: event.payload.text,
        time: now,
        is_outgoing: true,
        ts: Date.now(),
      };
      chat.reconcileSentMessageInThread(event.payload.thread_id, msg);
      ui.setStatusMessage(`Đã gửi tin nhắn lúc ${now}`);
      return;
    }

    case "StickerSent": {
      const now = formatClockNow();
      const finalId = event.payload.msg_id || `out_${event.payload.client_message_id}`;
      const msg: ChatMessage = {
        id: finalId,
        client_message_id: event.payload.client_message_id,
        sender: "Tôi",
        text: "[Nhãn dán]",
        time: now,
        is_outgoing: true,
        ts: Date.now(),
        sticker_id: event.payload.sticker_id,
        sticker_cate_id: event.payload.cate_id,
        sticker_type: event.payload.sticker_type,
      };
      chat.reconcileSentMessageInThread(event.payload.thread_id, msg);
      ui.setStatusMessage(`Đã gửi nhãn dán lúc ${now}`);
      return;
    }

    case "AttachmentSent":
      chat.reconcileSentMessageInThread(event.payload.thread_id, event.payload.msg);
      ui.setStatusMessage(`Đã gửi đính kèm lúc ${formatClockNow()}`);
      return;

    case "CallHistoryRecorded":
      chat.reconcileSentMessageInThread(event.payload.thread_id, event.payload.msg);
      return;

    case "QuoteSent": {
      const now = formatClockNow();
      const finalId = event.payload.msg_id || `out_${event.payload.client_message_id}`;
      const msg: ChatMessage = {
        id: finalId,
        client_message_id: event.payload.client_message_id,
        sender: "Tôi",
        text: event.payload.text,
        time: now,
        is_outgoing: true,
        ts: Date.now(),
        reply_to_id: event.payload.quoted.id,
        reply_to_sender: event.payload.quoted.sender,
        reply_to_text: event.payload.quoted.text,
      };
      chat.reconcileSentMessageInThread(event.payload.thread_id, msg);
      ui.setStatusMessage(`Đã gửi trả lời lúc ${now}`);
      return;
    }

    case "Reacted":
      chat.reactToMessage(
        event.payload.thread_id,
        event.payload.msg_id,
        event.payload.ts,
        event.payload.reaction,
        false,
      );
      ui.setStatusMessage("Đã thả cảm xúc");
      return;

    case "ReactFailed":
      chat.reactToMessage(
        event.payload.thread_id,
        event.payload.msg_id,
        event.payload.ts,
        event.payload.reaction,
        true,
      );
      ui.setStatusMessage("Thả cảm xúc thất bại");
      return;

    case "MessageDeleted":
      chat.deleteMessageLocal(event.payload.thread_id, event.payload.msg_id, event.payload.ts);
      ui.setStatusMessage("Đã xóa tin nhắn khỏi lịch sử máy này");
      return;

    case "MessageRecalled":
      chat.recallMessageLocal(event.payload.thread_id, event.payload.msg_id, event.payload.ts);
      ui.setStatusMessage("Đã thu hồi tin nhắn");
      return;

    case "SendFailed":
      ui.setStatusMessage(event.payload.error);
      ui.showToast(event.payload.error, "error");
      return;

    case "HistoryReachedStart":
      chat.setHistoryReachedStart(event.payload.thread_id, true);
      ui.showToast("Đã hiển thị toàn bộ lịch sử hội thoại này", "info");
      return;

    case "StickerImageReady":
      media.putMedia(event.payload.key, event.payload.data_url);
      return;

    case "StickerStoreLoaded":
      media.setStickerStorePacks(event.payload.packs);
      return;

    case "StickerPackItemsLoaded":
      media.setStickerPackItems(event.payload.cate_id, event.payload.items);
      return;

    case "StickerSearchResults":
      media.setStickerSearchResults(event.payload.items);
      return;

    case "StickerSuggestKeywordsLoaded":
      media.setStickerSuggestKeywords(event.payload.keywords);
      return;

    case "ComposerStickerSuggestions":
      media.setComposerStickerSuggestions(event.payload.items);
      return;

    case "LinkPreviewReady":
      media.setLinkMetadata(event.payload.meta.url, event.payload.meta);
      return;

    case "FileDownloaded":
      ui.showToast(
        event.payload.ok
          ? `Đã lưu tệp: ${event.payload.file_name} (${event.payload.path})`
          : `Tải tệp thất bại: ${event.payload.error ?? "không xác định"}`,
        event.payload.ok ? "success" : "error",
      );
      return;

    case "LocalDataWiped":
      chat.clearAllMessagesLocal();
      media.resetMediaCache();
      ui.showToast("Đã xóa dữ liệu tin nhắn cục bộ", "info");
      ui.setStatusMessage("Đã xóa dữ liệu tin nhắn cục bộ");
      return;

    case "StatusUpdate":
      ui.setStatusMessage(event.payload.message);
      return;

    case "RefreshFinished": {
      const msg =
        event.payload.errors.length === 0
          ? `Đã làm mới ${event.payload.threads} hội thoại (${event.payload.messages} tin nhắn)`
          : `Làm mới ${event.payload.threads} hội thoại · ${event.payload.errors.length} lỗi`;
      ui.setStatusMessage(msg);
      if (event.payload.manual) ui.showToast(msg, "info");
      return;
    }

    case "SyncFinished": {
      const msg = event.payload.cancelled
        ? `Đã dừng đồng bộ (đã thêm ${event.payload.added} tin nhắn)`
        : `Đồng bộ hoàn tất: +${event.payload.added} tin nhắn`;
      chat.setIsSyncing(false);
      ui.setStatusMessage(msg);
      ui.showToast(msg, "info");
      return;
    }

    case "PhoneSyncProgress":
      chat.setPhoneSyncRunning(true);
      chat.setPhoneSyncProgress({
        current: event.payload.progress,
        total: 100,
        stage: event.payload.status,
      });
      return;

    case "PhoneSyncFinished": {
      const msg = event.payload.error
        ? `Đồng bộ điện thoại lỗi: ${event.payload.error}`
        : event.payload.cancelled
          ? `Đã dừng đồng bộ điện thoại (+${event.payload.added} tin nhắn)`
          : `Đồng bộ điện thoại hoàn tất (+${event.payload.added} tin nhắn từ ${event.payload.active_threads} hội thoại)`;
      const sorted = chat.settings
        ? sortThreads(event.payload.threads, chat.settings)
        : event.payload.threads;
      chat.setPhoneSyncRunning(false);
      chat.setPhoneSyncProgress({
        current: event.payload.error ? 0 : 1,
        total: 1,
        stage: msg,
      });
      if (sorted.length > 0) chat.setThreads(sorted);
      ui.showToast(msg, event.payload.error ? "error" : "info");
      return;
    }

    case "MuteSnapshotSynced":
      chat.syncMuteSnapshot(event.payload.muted_threads);
      return;

    case "MuteEventsReceived":
      chat.applyMuteEvents(event.payload.events);
      return;

    case "NotificationAuthorization":
      if (event.payload.status_label ?? event.payload.error) {
        ui.setStatusMessage(event.payload.status_label ?? event.payload.error ?? "");
      }
      return;

    case "UpdateCheckFinished":
      ui.setUpdateCheckFinished(
        event.payload.ok,
        event.payload.info,
        event.payload.error ?? null,
        event.payload.silent,
      );
      return;

    case "UpdateDownloadStatus":
      ui.setUpdateDownloadStatus(event.payload.downloading, event.payload.message);
      return;

    case "UpdateDownloadProgress":
      ui.setUpdateDownloadProgress({
        downloaded: event.payload.downloaded,
        total: event.payload.total,
      });
      return;

    case "UpdateAppliedAndRestarting":
      ui.setUpdateApplied(event.payload.version);
      return;

    case "PhoneLookupSuccess":
      contacts.setPhoneLookupResult(event.payload.profile);
      if (event.payload.profile.friendship) {
        contacts.setFriendStatus(event.payload.profile.user_id, event.payload.profile.friendship);
      }
      return;

    case "PhoneLookupFailed":
      contacts.setPhoneLookupError(event.payload.error);
      return;

    case "FriendStatusChecked":
      contacts.setFriendStatus(event.payload.user_id, event.payload.status);
      if (event.payload.online) {
        contacts.setUserOnline(event.payload.user_id, event.payload.online);
      }
      return;

    case "CallStateChanged":
      useCallStore.getState().applyCallState(event.payload);
      return;
  }
}

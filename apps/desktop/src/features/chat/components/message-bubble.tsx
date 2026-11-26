import { Star } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import type { ChatMessage, Thread } from "@/shared/types/ipc.generated";
import {
  ChatBubble,
  ChatBubbleAvatar,
  ChatBubbleMessage,
  ChatBubbleTimestamp,
} from "@/shared/ui/chat";
import { cn } from "cn";
import { useChatStore, useMediaCacheStore, useUIStore } from "@/stores";
import {
  isFileMessage,
  isGifUrl,
  isPhotoMessage,
  isVideoMessage,
  parseCallCard,
} from "@/features/chat/utils/messages";
import {
  CallCardMedia,
  FileMessageMedia,
  LinkPreviewCard,
  PhotoMessageMedia,
  StickerMessageMedia,
  VideoMessageMedia,
} from "./message-media";
import { ContextMenu, ContextMenuTrigger } from "@/shared/ui/context-menu";
import { MessageReactionsChip } from "./message-reactions";
import { MessageContextMenu } from "./message-actions";
import type { ResolvedSender } from "./use-sender-lookup";

interface MessageBubbleProps {
  msg: ChatMessage;
  thread: Thread;
  isGroupedWithPrev: boolean;
  highlighted: boolean;
  senderInfo: ResolvedSender | null;
  jumpToQuoted: (msg: ChatMessage) => void;
  resolveReplySenderName: (raw?: string | null) => string;
  setRef: (el: HTMLDivElement | null) => void;
}

export function MessageBubble({
  msg,
  thread,
  isGroupedWithPrev,
  highlighted,
  senderInfo,
  jumpToQuoted,
  resolveReplySenderName,
  setRef,
}: MessageBubbleProps) {
  const doubleClickToReply = useChatStore((s) => s.settings?.double_click_to_reply ?? false);
  const downloadDir = useChatStore((s) => s.settings?.download_dir ?? "");
  const setReplyingTo = useChatStore((s) => s.setReplyingTo);
  const reactToMessage = useChatStore((s) => s.reactToMessage);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);
  const inlineVideoSupported = useUIStore((s) => s.inlineVideoSupported);
  const inlineVideoReason = useUIStore((s) => s.inlineVideoReason);
  const showToast = useUIStore((s) => s.showToast);
  const setImageViewerState = useUIStore((s) => s.setImageViewerState);
  const setStatusMessage = useUIStore((s) => s.setStatusMessage);

  const isRecalled = msg.text.trim() === "[Tin nhắn đã thu hồi]";
  const callCard = parseCallCard(msg);
  const isBareSticker = Boolean(msg.sticker_id);
  const isTextOrRich = !isBareSticker;
  const isPhotoBubble =
    !isRecalled &&
    !msg.sticker_id &&
    !callCard &&
    isPhotoMessage(msg) &&
    Boolean(msg.link_thumb ?? msg.link_url ?? isGifUrl(msg.text));
  const isFileBubble =
    !isRecalled && !msg.sticker_id && !callCard && isFileMessage(msg) && Boolean(msg.link_url);
  const isFlushMedia = (isPhotoBubble || isFileBubble) && !msg.reply_to_text;
  const bubbleVariant = msg.is_outgoing ? "sent" : "received";

  const handleReaction = (reaction: string) => {
    reactToMessage(thread.id, msg.id, msg.ts, reaction, false);
    setStatusMessage("Đã thả cảm xúc");
    void ipc.reactMessage(thread.id, thread.is_group, msg, reaction);
  };

  const bubbleContent = (
    <ChatBubble
      ref={setRef}
      variant={bubbleVariant}
      onDoubleClick={() => {
        if (doubleClickToReply && !isRecalled) {
          setReplyingTo(msg);
        }
      }}
      className={cn(highlighted && "rounded-lg bg-primary/10 p-1 ring-2 ring-primary/60")}
    >
      {/* ── Avatar for received messages ── */}
      {!msg.is_outgoing && senderInfo ? (
        !isGroupedWithPrev ? (
          <ChatBubbleAvatar
            id={senderInfo.id}
            name={senderInfo.name}
            avatarUrl={senderInfo.avatar}
            size={32}
            mediaCache={mediaCache}
          />
        ) : (
          <div className="w-8 shrink-0" />
        )
      ) : null}

      {/* ── Bubble column ── */}
      <div
        className={cn(
          "relative flex min-w-0 flex-col",
          msg.is_outgoing ? "items-end" : "items-start",
        )}
      >
        {/* ── shadcn-chat ChatBubbleMessage ── */}
        <ChatBubbleMessage
          variant={bubbleVariant}
          className={cn(
            isBareSticker && "border-none bg-transparent p-0 shadow-none",
            isFlushMedia && "overflow-hidden p-0",
            isRecalled && "italic opacity-70",
          )}
        >
          {/* Sender display name for group chats */}
          {!msg.is_outgoing &&
          thread.is_group &&
          !isGroupedWithPrev &&
          senderInfo &&
          isTextOrRich &&
          !isRecalled ? (
            <div
              className={cn(
                "text-xs font-semibold text-foreground select-none",
                isFlushMedia ? "px-3 pt-2 pb-1" : "mb-1",
              )}
            >
              {senderInfo.name}
            </div>
          ) : null}

          {/* Reply quote snippet */}
          {msg.reply_to_text ? (
            <div
              onClick={() => jumpToQuoted(msg)}
              className="mb-2 flex cursor-pointer items-stretch gap-2 rounded-md border border-border bg-background px-2.5 py-1.5 text-xs hover:bg-accent/50"
            >
              <div className="w-0.5 shrink-0 rounded-full bg-primary" />
              <div className="flex min-w-0 flex-col">
                <span className="text-[11px] font-semibold text-foreground">
                  {resolveReplySenderName(msg.reply_to_sender)}
                </span>
                <span className="max-w-75 truncate text-muted-foreground">{msg.reply_to_text}</span>
              </div>
            </div>
          ) : null}

          {/* Message content dispatcher */}
          {isRecalled ? (
            <span>Tin nhắn đã được thu hồi</span>
          ) : msg.sticker_id ? (
            <StickerMessageMedia msg={msg} />
          ) : callCard ? (
            <CallCardMedia callCard={callCard} />
          ) : isVideoMessage(msg) && msg.link_url ? (
            <VideoMessageMedia
              msg={msg}
              downloadDir={downloadDir}
              inlineVideoSupported={inlineVideoSupported}
              inlineVideoReason={inlineVideoReason}
              onNotify={(message) => showToast(message)}
            />
          ) : isPhotoBubble ? (
            <PhotoMessageMedia
              msg={msg}
              flush={isFlushMedia}
              onOpenViewer={(texKey, fullUrl) =>
                setImageViewerState({
                  texKey,
                  fullUrl,
                  title: msg.link_title ?? "Hình ảnh",
                  sender: senderInfo?.name ?? msg.sender,
                  time: msg.time,
                })
              }
            />
          ) : isFileBubble ? (
            <FileMessageMedia msg={msg} downloadDir={downloadDir} flush={isFlushMedia} />
          ) : (
            <div>
              <div className="select-text">{msg.text}</div>
              <LinkPreviewCard msg={msg} />
            </div>
          )}

          {/* Inline Timestamp & delivery status */}
          {isTextOrRich && !isRecalled ? (
            <ChatBubbleTimestamp
              timestamp={msg.time}
              className={cn(
                isFlushMedia && "px-3 pb-2",
                msg.is_outgoing
                  ? "justify-end text-bubble-out-foreground/60"
                  : "text-muted-foreground",
              )}
            >
              {msg.is_starred ? <Star className="h-3 w-3 text-warning" /> : null}
              {msg.is_outgoing &&
              ((msg.delivery as string) === "seen" || msg.delivery === "Read") ? (
                <span className="opacity-80">· Đã xem</span>
              ) : msg.is_outgoing &&
                ((msg.delivery as string) === "sent" ||
                  msg.delivery === "ServerAccepted" ||
                  msg.delivery === "Delivered") ? (
                <span className="opacity-60">· Đã gửi</span>
              ) : null}
            </ChatBubbleTimestamp>
          ) : null}
        </ChatBubbleMessage>

        {/* Timestamp below for stickers & recalled messages */}
        {isBareSticker || isRecalled ? (
          <div className="flex items-center gap-1 px-1 pt-0.5 text-xs text-muted-foreground select-none">
            <span>{msg.time}</span>
            {msg.is_starred ? <Star className="h-3 w-3 text-warning" /> : null}
          </div>
        ) : null}

        {/* Reaction badge/chip */}
        <MessageReactionsChip reactions={msg.reactions} isOutgoing={msg.is_outgoing} />
      </div>
    </ChatBubble>
  );

  if (isRecalled) return bubbleContent;

  return (
    <ContextMenu>
      <ContextMenuTrigger render={bubbleContent} />
      <MessageContextMenu msg={msg} thread={thread} onSelectReaction={handleReaction} />
    </ContextMenu>
  );
}

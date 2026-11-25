import { Users } from "lucide-react";
import { Avatar, AvatarBadge, AvatarFallback, AvatarImage } from "@/shared/ui/avatar";
import { cn } from "cn";
import { avatarColor, initialsOf, jxlToJpgUrl } from "@/shared/utils/text";

interface UserAvatarProps {
  id: string;
  name: string;
  avatarUrl?: string | null;
  isGroup?: boolean;
  memberCount?: number;
  size?: number;
  mediaCache?: Record<string, string | undefined>;
  className?: string;
}

/** Domain avatar wrapper built on Base UI `Avatar` with cached media and group badge support. */
export function UserAvatar({
  id,
  name,
  avatarUrl,
  isGroup = false,
  memberCount,
  size = 38,
  mediaCache,
  className,
}: UserAvatarProps) {
  const cachedDataUrl = mediaCache?.[`ava_${id}`];
  const src = cachedDataUrl || (avatarUrl ? jxlToJpgUrl(avatarUrl) : "");

  return (
    <Avatar
      style={{ width: size, height: size }}
      className={cn("shrink-0 overflow-visible", className)}
    >
      {src ? <AvatarImage src={src} alt={name} loading="lazy" decoding="async" /> : null}
      <AvatarFallback
        style={{
          backgroundColor: avatarColor(name || id),
          fontSize: Math.max(11, Math.round(size * 0.36)),
        }}
        className="font-bold text-white"
      >
        {isGroup ? <Users size={Math.round(size * 0.5)} /> : initialsOf(name || "Z")}
      </AvatarFallback>
      {isGroup && memberCount && memberCount > 0 && size >= 36 ? (
        <AvatarBadge className="-right-1 -bottom-1 h-4 min-w-4 bg-card px-1 text-[10px] leading-none font-semibold text-muted-foreground ring-1 ring-border">
          <Users className="size-2.5" />
          <span>{memberCount > 999 ? "999+" : memberCount}</span>
        </AvatarBadge>
      ) : null}
    </Avatar>
  );
}

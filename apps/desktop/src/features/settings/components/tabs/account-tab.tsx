import { Trash2 } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import { Avatar } from "@/shared/ui/avatar";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import { useAuthStore, useMediaCacheStore, useUIStore } from "@/stores";
import { RowDivider, SectionHeading, SettingCard } from "../primitives";

export function AccountTab() {
  const uin = useAuthStore((s) => s.uin);
  const displayName = useAuthStore((s) => s.displayName);
  const avatar = useAuthStore((s) => s.avatar);
  const savedAccounts = useAuthStore((s) => s.savedAccounts);
  const setSavedAccounts = useAuthStore((s) => s.setSavedAccounts);
  const setSettingsOpen = useUIStore((s) => s.setSettingsOpen);
  const mediaCache = useMediaCacheStore((s) => s.mediaCache);

  return (
    <>
      <SectionHeading>Tài khoản hiện tại</SectionHeading>
      <SettingCard>
        <div className="flex items-center justify-between px-4 py-3">
          <div className="flex items-center gap-3">
            <Avatar
              id={uin || "self"}
              name={displayName || "Me"}
              avatarUrl={avatar}
              size={44}
              mediaCache={mediaCache}
            />
            <div>
              <div className="text-sm font-medium text-foreground">
                {displayName || "Người dùng Zalo"}
              </div>
              <div className="text-xs text-muted-foreground">
                {uin ? `UIN: ${uin}` : "Đang hoạt động"}
              </div>
            </div>
          </div>
          <Button variant="destructive" size="sm" onClick={() => void ipc.logout()}>
            Đăng xuất
          </Button>
        </div>
      </SettingCard>

      <SectionHeading>Đa tài khoản</SectionHeading>
      <SettingCard>
        <div className="flex items-center justify-between px-4 py-3">
          <span className="text-sm text-foreground">Tài khoản đã lưu</span>
          <Button
            variant="default"
            size="sm"
            onClick={() => {
              setSettingsOpen(false);
              void ipc.addNewAccount();
            }}
          >
            + Thêm tài khoản (QR)
          </Button>
        </div>
        {savedAccounts.length === 0 ? (
          <>
            <RowDivider />
            <div className="px-4 py-3 text-xs text-muted-foreground">
              Chưa có tài khoản phụ nào được lưu trong thư mục accounts/.
            </div>
          </>
        ) : (
          savedAccounts.map((acc) => (
            <div key={acc.uin}>
              <RowDivider />
              <div className="flex items-center justify-between px-4 py-2.5">
                <div className="flex items-center gap-2.5">
                  <Avatar
                    id={acc.uin}
                    name={acc.display_name}
                    avatarUrl={acc.avatar}
                    size={32}
                    mediaCache={mediaCache}
                  />
                  <div>
                    <div className="text-sm font-medium text-foreground">{acc.display_name}</div>
                    <div className="text-xs text-muted-foreground">UIN: {acc.uin}</div>
                  </div>
                </div>
                <div className="flex items-center gap-1.5">
                  {acc.is_current ? (
                    <Badge variant="online">Đang dùng</Badge>
                  ) : (
                    <>
                      <Button
                        variant="default"
                        size="sm"
                        onClick={() => void ipc.switchAccount(acc.uin)}
                      >
                        Chuyển đổi
                      </Button>
                      <Button
                        variant="destructive"
                        size="icon-sm"
                        onClick={async () => {
                          await ipc.deleteSavedAccount(acc.uin);
                          const next = await ipc.listSavedAccounts();
                          setSavedAccounts(next);
                        }}
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    </>
                  )}
                </div>
              </div>
            </div>
          ))
        )}
      </SettingCard>
    </>
  );
}

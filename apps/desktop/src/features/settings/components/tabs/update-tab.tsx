import { Download, RefreshCw } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import { Button } from "@/shared/ui/button";
import { useAuthStore, useUIStore } from "@/stores";
import { RowDivider, SectionHeading, SettingCard, SwitchRow } from "../primitives";
import type { SettingsTabProps } from "../types";

export function UpdateTab({ settings, updateSettings }: SettingsTabProps) {
  const appVersion = useAuthStore((s) => s.appVersion);
  const updateChecking = useUIStore((s) => s.updateChecking);
  const updateInfo = useUIStore((s) => s.updateInfo);
  const updateDownloading = useUIStore((s) => s.updateDownloading);
  const updateDownloadMessage = useUIStore((s) => s.updateDownloadMessage);
  const setUpdateChecking = useUIStore((s) => s.setUpdateChecking);

  return (
    <>
      <SectionHeading>Cập nhật ứng dụng</SectionHeading>
      <SettingCard>
        <div className="flex items-center justify-between px-4 py-3">
          <div>
            <div className="text-sm font-medium text-foreground">AloLite v{appVersion}</div>
            <div className="text-xs text-muted-foreground">
              {updateChecking
                ? "Đang kiểm tra bản phát hành mới…"
                : updateInfo?.has_update
                  ? `Có bản mới v${updateInfo.latest_version}`
                  : "Kiểm tra cập nhật trực tiếp từ GitHub Releases"}
            </div>
          </div>
          <Button
            variant="default"
            size="sm"
            disabled={updateChecking}
            onClick={() => {
              setUpdateChecking(true);
              void ipc.checkForUpdates(false);
            }}
          >
            <RefreshCw className="h-3.5 w-3.5" /> Kiểm tra ngay
          </Button>
        </div>
        {updateInfo?.has_update && updateInfo.download_url ? (
          <>
            <RowDivider />
            <div className="px-4 py-3">
              <Button
                variant="default"
                disabled={updateDownloading}
                onClick={() =>
                  void ipc.downloadAndApplyUpdate(
                    updateInfo.download_url!,
                    updateInfo.asset_name ?? "alolite",
                    updateInfo.latest_version,
                    updateInfo.expected_sha256,
                  )
                }
              >
                <Download className="h-3.5 w-3.5" /> Tải và cài đặt v{updateInfo.latest_version}
              </Button>
            </div>
          </>
        ) : null}
        {updateDownloadMessage ? (
          <>
            <RowDivider />
            <div className="px-4 py-2 text-xs text-muted-foreground">{updateDownloadMessage}</div>
          </>
        ) : null}
      </SettingCard>

      <SectionHeading>Tự động cập nhật</SectionHeading>
      <SettingCard>
        <SwitchRow
          label="Tự động kiểm tra cập nhật"
          checked={settings.auto_check_updates}
          onChange={(v) => updateSettings({ auto_check_updates: v })}
        />
      </SettingCard>
    </>
  );
}

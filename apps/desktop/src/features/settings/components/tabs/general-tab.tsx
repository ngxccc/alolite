import { FolderOpen } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import { Button } from "@/shared/ui/button";
import { SectionHeading, SettingCard } from "../primitives";
import type { SettingsTabProps } from "../types";

export function GeneralTab({ settings, updateSettings }: SettingsTabProps) {
  return (
    <>
      <SectionHeading>Thư mục tải xuống</SectionHeading>
      <SettingCard>
        <div className="px-4 py-3">
          <div className="mb-2 text-xs text-muted-foreground">
            Lưu tại:{" "}
            <code className="rounded bg-muted px-1.5 py-0.5 text-foreground">
              {settings.download_dir}
            </code>
          </div>
          <div className="flex gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={async () => {
                const dir = await ipc.pickFile({
                  directory: true,
                  title: "Chọn thư mục lưu file",
                });
                if (dir) updateSettings({ download_dir: dir });
              }}
            >
              <FolderOpen className="h-3.5 w-3.5" /> Thay đổi thư mục
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => void ipc.openPathInOs(settings.download_dir)}
            >
              Mở thư mục
            </Button>
          </div>
        </div>
      </SettingCard>
    </>
  );
}

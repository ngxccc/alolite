import { Trash2 } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogMedia,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { useUIStore } from "@/stores";

export function WipeConfirmModal() {
  const wipeConfirmOpen = useUIStore((s) => s.wipeConfirmOpen);
  const setWipeConfirmOpen = useUIStore((s) => s.setWipeConfirmOpen);

  if (!wipeConfirmOpen) return null;

  return (
    <AlertDialog open={wipeConfirmOpen} onOpenChange={(open) => !open && setWipeConfirmOpen(false)}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogMedia className="bg-destructive/10 text-destructive">
            <Trash2 className="h-5 w-5" />
          </AlertDialogMedia>
          <AlertDialogTitle>Xóa dữ liệu tin nhắn cục bộ</AlertDialogTitle>
          <AlertDialogDescription>
            Xóa{" "}
            <code className="rounded bg-muted px-1.5 py-0.5 text-xs text-foreground">
              zalo_history.db
            </code>{" "}
            và{" "}
            <code className="rounded bg-muted px-1.5 py-0.5 text-xs text-foreground">
              threads_cache.json
            </code>{" "}
            trên máy này. Không thể hoàn tác, nhưng phiên đăng nhập{" "}
            <code className="rounded bg-muted px-1.5 py-0.5 text-xs text-foreground">
              session.json
            </code>{" "}
            vẫn được giữ nguyên.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel onClick={() => setWipeConfirmOpen(false)}>Hủy</AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            onClick={() => {
              setWipeConfirmOpen(false);
              void ipc.wipeLocalData();
            }}
          >
            Xóa dữ liệu
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

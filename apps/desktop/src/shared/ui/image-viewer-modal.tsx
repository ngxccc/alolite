import { useEffect, useRef, useState } from "react";
import { Download, RotateCcw, X, ZoomIn, ZoomOut } from "lucide-react";
import { ipc } from "@/shared/api/ipc";
import { jxlToJpgUrl } from "@/shared/utils/text";
import { cn } from "cn";
import { Button } from "./button";
import { Dialog, DialogContent } from "./dialog";
import type { ImageViewerState } from "@/stores";

interface ImageViewerModalProps {
  viewer: ImageViewerState | null;
  mediaCache: Record<string, string | undefined>;
  downloadDir: string;
  onClose: () => void;
}

const MIN_ZOOM = 0.25;
const MAX_ZOOM = 5;

function clampZoom(val: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Number(val.toFixed(2))));
}

export function ImageViewerModal({
  viewer,
  mediaCache,
  downloadDir,
  onClose,
}: ImageViewerModalProps) {
  const [zoom, setZoom] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [dragging, setDragging] = useState(false);
  const dragOriginRef = useRef<{ x: number; y: number; startX: number; startY: number } | null>(
    null,
  );

  useEffect(() => {
    setZoom(1);
    setPan({ x: 0, y: 0 });
    setDragging(false);
    dragOriginRef.current = null;
  }, [viewer?.texKey, viewer?.fullUrl]);

  if (!viewer) return null;

  const hdKey = `hd_${viewer.texKey}`;
  const src = mediaCache[hdKey] ?? mediaCache[viewer.texKey] ?? jxlToJpgUrl(viewer.fullUrl);

  const resetView = () => {
    setZoom(1);
    setPan({ x: 0, y: 0 });
  };

  const applyZoom = (nextZoom: number) => {
    const clamped = clampZoom(nextZoom);
    setZoom(clamped);
    if (clamped <= 1) {
      setPan({ x: 0, y: 0 });
    }
  };

  const handleWheel = (e: React.WheelEvent<HTMLDivElement>) => {
    if (!src) return;
    e.preventDefault();
    const delta = e.deltaY < 0 ? 0.15 : -0.15;
    applyZoom(zoom * (1 + delta));
  };

  const handlePointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!src || e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    setDragging(true);
    dragOriginRef.current = {
      x: e.clientX,
      y: e.clientY,
      startX: pan.x,
      startY: pan.y,
    };
  };

  const handlePointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging || !dragOriginRef.current) return;
    const dx = e.clientX - dragOriginRef.current.x;
    const dy = e.clientY - dragOriginRef.current.y;
    setPan({
      x: dragOriginRef.current.startX + dx,
      y: dragOriginRef.current.startY + dy,
    });
  };

  const handlePointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging) return;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    setDragging(false);
    dragOriginRef.current = null;
  };

  const handleDoubleClick = () => {
    if (!src) return;
    if (zoom !== 1) {
      resetView();
    } else {
      setZoom(2);
    }
  };

  return (
    <Dialog open={Boolean(viewer)} onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        showCloseButton={false}
        className="flex h-[86vh] max-h-[86vh] w-[88vw] max-w-[88vw] flex-col gap-0 overflow-hidden bg-background p-0 sm:max-w-[88vw]"
      >
        <div className="flex h-14 shrink-0 items-center justify-between border-b border-border bg-sidebar px-4">
          <div className="min-w-0">
            <div className="truncate text-sm font-semibold">{viewer.title || "Hình ảnh"}</div>
            <div className="truncate text-xs text-muted-foreground">
              {viewer.sender} · {viewer.time}
            </div>
          </div>

          <div className="flex items-center gap-1.5">
            <Button
              variant="outline"
              size="icon-sm"
              aria-label="Thu nhỏ"
              title="Thu nhỏ"
              onClick={() => applyZoom(zoom - 0.25)}
            >
              <ZoomOut className="h-3.5 w-3.5" aria-hidden="true" />
            </Button>
            <span className="min-w-12 text-center font-mono text-xs text-muted-foreground tabular-nums">
              {Math.round(zoom * 100)}%
            </span>
            <Button
              variant="outline"
              size="icon-sm"
              aria-label="Phóng to"
              title="Phóng to"
              onClick={() => applyZoom(zoom + 0.25)}
            >
              <ZoomIn className="h-3.5 w-3.5" aria-hidden="true" />
            </Button>
            <Button
              variant="outline"
              size="icon-sm"
              aria-label="Đặt lại"
              title="Đặt lại"
              onClick={resetView}
            >
              <RotateCcw className="h-3.5 w-3.5" aria-hidden="true" />
            </Button>
            {viewer.fullUrl ? (
              <Button
                variant="default"
                size="sm"
                onClick={() =>
                  void ipc.downloadFile(
                    viewer.fullUrl,
                    viewer.title || "zalo_image.jpg",
                    downloadDir,
                  )
                }
              >
                <Download className="h-3.5 w-3.5" aria-hidden="true" /> Tải xuống
              </Button>
            ) : null}
            <Button
              variant="ghost"
              size="icon-sm"
              onClick={onClose}
              aria-label="Đóng"
              title="Đóng (Esc)"
            >
              <X className="h-4 w-4" aria-hidden="true" />
            </Button>
          </div>
        </div>

        <div
          onWheel={handleWheel}
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={handlePointerUp}
          onPointerCancel={handlePointerUp}
          onDoubleClick={handleDoubleClick}
          className={cn(
            "flex flex-1 items-center justify-center overflow-hidden bg-black/80 p-6 select-none",
            src ? (dragging ? "cursor-grabbing" : "cursor-grab") : "cursor-default",
          )}
        >
          {src ? (
            <img
              src={src}
              alt={viewer.title}
              draggable={false}
              style={{
                transform: `translate3d(${pan.x}px, ${pan.y}px, 0) scale(${zoom})`,
              }}
              className={cn(
                "max-h-full max-w-full origin-center rounded-md object-contain",
                !dragging && "transition-transform duration-100",
              )}
            />
          ) : (
            <span className="text-xs text-muted-foreground">Đang tải ảnh độ phân giải cao…</span>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

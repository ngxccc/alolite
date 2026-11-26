import { ArrowDown } from "lucide-react";
import { Button } from "@/shared/ui/button";
import { cn } from "cn";
import { forwardRef, useCallback, useEffect, useRef, useState } from "react";

export interface ChatMessageListProps extends React.HTMLAttributes<HTMLDivElement> {
  smooth?: boolean;
  showScrollToBottom?: boolean;
}

export const ChatMessageList = forwardRef<HTMLDivElement, ChatMessageListProps>(
  ({ className, children, smooth = false, showScrollToBottom = true, ...props }, ref) => {
    const scrollElRef = useRef<HTMLDivElement | null>(null);
    const contentRef = useRef<HTMLDivElement | null>(null);
    const isAtBottomRef = useRef(true);
    const [isAtBottom, setIsAtBottom] = useState(true);

    const setMergedRef = useCallback(
      (node: HTMLDivElement | null) => {
        scrollElRef.current = node;
        if (typeof ref === "function") {
          ref(node);
        } else if (ref) {
          ref.current = node;
        }
      },
      [ref],
    );

    const handleScroll = (e: React.UIEvent<HTMLDivElement>) => {
      props.onScroll?.(e);
      const target = e.currentTarget;
      const atBottom = target.scrollHeight - target.scrollTop - target.clientHeight < 48;
      isAtBottomRef.current = atBottom;
      setIsAtBottom(atBottom);
    };

    useEffect(() => {
      const content = contentRef.current;
      const scroller = scrollElRef.current;
      if (!content || !scroller || typeof ResizeObserver === "undefined") return;

      const observer = new ResizeObserver(() => {
        if (isAtBottomRef.current) {
          scroller.scrollTop = scroller.scrollHeight;
        }
      });
      observer.observe(content);
      return () => observer.disconnect();
    }, []);

    const scrollToBottom = () => {
      const container = scrollElRef.current;
      if (container) {
        isAtBottomRef.current = true;
        setIsAtBottom(true);
        container.scrollTo({
          top: container.scrollHeight,
          behavior: smooth ? "smooth" : "auto",
        });
      }
    };

    return (
      <div className="relative flex min-h-0 w-full flex-1 flex-col">
        <div
          ref={setMergedRef}
          onScroll={handleScroll}
          className={cn("flex flex-1 flex-col overflow-y-auto px-4 py-3", className)}
          {...props}
        >
          <div ref={contentRef} className="mt-auto flex flex-col gap-0.5">
            {children}
          </div>
        </div>

        {showScrollToBottom ? (
          <Button
            onClick={scrollToBottom}
            size="icon"
            variant="outline"
            className={cn(
              "absolute right-6 bottom-3 z-30 h-8 w-8 rounded-full border-border bg-popover/90 text-foreground shadow-md backdrop-blur-xs transition-all duration-150 hover:bg-popover",
              isAtBottom ? "pointer-events-none translate-y-1 opacity-0" : "opacity-100",
            )}
            aria-label="Cuộn xuống cuối"
          >
            <ArrowDown className="h-4 w-4" />
          </Button>
        ) : null}
      </div>
    );
  },
);
ChatMessageList.displayName = "ChatMessageList";

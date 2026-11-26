import type * as React from "react";
import { cn } from "cn";

type SliderProps = Omit<React.ComponentProps<"input">, "type">;

function Slider({ className, ...props }: SliderProps) {
  return (
    <input
      type="range"
      data-slot="slider"
      className={cn(
        "h-1.5 w-full cursor-pointer appearance-none rounded-full bg-muted accent-emerald-500 outline-none focus-visible:ring-2 focus-visible:ring-ring",
        className,
      )}
      {...props}
    />
  );
}

export { Slider };

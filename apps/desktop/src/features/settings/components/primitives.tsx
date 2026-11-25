import type React from "react";
import { ChevronRight } from "lucide-react";
import { Card } from "@/shared/ui/card";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/shared/ui/select";
import { Switch } from "@/shared/ui/switch";
import { cn } from "cn";

/** Zalo-style row: label+optional desc on left, toggle on right */
export function SwitchRow({
  label,
  desc,
  checked,
  onChange,
}: {
  label: string;
  desc?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex min-h-[44px] items-center justify-between gap-4 px-4 py-2">
      <div className="min-w-0 flex-1">
        <div className="text-sm text-foreground">{label}</div>
        {desc ? <div className="mt-0.5 text-xs text-muted-foreground">{desc}</div> : null}
      </div>
      {/* 40x22 toggle */}
      <Switch size="lg" checked={checked} onCheckedChange={onChange} />
    </div>
  );
}

/** Select row */
export function SelectRow({
  label,
  value,
  onChange,
  options,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
}) {
  return (
    <div className="flex min-h-[44px] items-center justify-between gap-4 px-4 py-2">
      <span className="text-sm text-foreground">{label}</span>
      <Select value={value} items={options} onValueChange={(v) => v && onChange(v)}>
        <SelectTrigger className="h-[38px] min-w-[160px] border-border bg-canvas text-sm">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {options.map((o) => (
            <SelectItem key={o.value} value={o.value}>
              {o.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

/** Chevron-only nav row (block messages, manage quick messages, etc.) */
export function ChevronRow({ label, desc }: { label: string; desc?: string }) {
  return (
    <div className="flex min-h-[44px] cursor-pointer items-center justify-between gap-4 px-4 py-2 hover:bg-white/[0.03]">
      <div className="min-w-0 flex-1">
        <div className="text-sm text-foreground">{label}</div>
        {desc ? <div className="mt-0.5 text-xs text-muted-foreground">{desc}</div> : null}
      </div>
      <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
    </div>
  );
}

/** Section heading (not inside a card) */
export function SectionHeading({ children }: { children: React.ReactNode }) {
  return (
    <div className="mt-4 mb-2 text-base font-medium text-foreground first:mt-0">{children}</div>
  );
}

/** Card wrapper */
export function SettingCard({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return <Card className={cn("gap-0 overflow-hidden py-0", className)}>{children}</Card>;
}

/** Divider between rows inside card */
export function RowDivider() {
  return <div className="mx-4 h-px bg-border" />;
}

/** CSS mini-mockup for theme picker (Zinc Light / Dark / System) */
export function ThemeThumbnail({ mode }: { mode: "light" | "dark" | "system" }) {
  if (mode === "light") {
    return (
      <div className="relative h-[72px] w-full overflow-hidden rounded border border-[#e4e4e7] bg-[#fafafa]">
        <div className="absolute inset-y-0 left-0 w-[30%] border-r border-[#e4e4e7] bg-white" />
        <div className="absolute right-3 bottom-3 h-[12px] w-[45%] rounded-sm bg-[#18181b]" />
        <div className="absolute bottom-7 left-[32%] h-[12px] w-[40%] rounded-sm border border-[#e4e4e7] bg-white" />
        <div className="absolute inset-x-0 top-0 h-[14px] border-b border-[#e4e4e7] bg-white" />
      </div>
    );
  }
  if (mode === "dark") {
    return (
      <div className="relative h-[72px] w-full overflow-hidden rounded border border-[#27272a] bg-[#09090b]">
        <div className="absolute inset-y-0 left-0 w-[30%] border-r border-[#27272a] bg-[#09090b]" />
        <div className="absolute right-3 bottom-3 h-[12px] w-[45%] rounded-sm bg-[#27272a]" />
        <div className="absolute bottom-7 left-[32%] h-[12px] w-[40%] rounded-sm bg-[#18181b]" />
        <div className="absolute inset-x-0 top-0 h-[14px] border-b border-[#27272a] bg-[#09090b]" />
      </div>
    );
  }
  // system: half-and-half
  return (
    <div className="relative h-[72px] w-full overflow-hidden rounded border border-border">
      {/* left half dark */}
      <div className="absolute inset-y-0 left-0 w-1/2 bg-[#09090b]">
        <div className="absolute inset-y-0 left-0 w-[35%] border-r border-[#27272a] bg-[#09090b]" />
        <div className="absolute inset-x-0 top-0 h-[14px] border-b border-[#27272a] bg-[#09090b]" />
      </div>
      {/* right half light */}
      <div className="absolute inset-y-0 right-0 w-1/2 bg-[#fafafa]">
        <div className="absolute inset-x-0 top-0 h-[14px] border-b border-[#e4e4e7] bg-white" />
        <div className="absolute right-2 bottom-3 h-[10px] w-[55%] rounded-sm bg-[#18181b]" />
      </div>
    </div>
  );
}

/** CSS mini-mockup for notification toggle (laptop outline) */
export function NotifThumbnail({ active }: { active: boolean }) {
  return (
    <div
      className={cn(
        "relative mx-auto flex h-[60px] w-[80px] flex-col items-center justify-center rounded border-2",
        active ? "border-primary" : "border-border",
      )}
    >
      {/* laptop screen */}
      <div
        className={cn(
          "h-[32px] w-[56px] rounded-t border-2",
          active ? "border-primary" : "border-muted-foreground",
        )}
      >
        {active && <div className="mt-1 ml-1 h-2 w-6 rounded-sm bg-primary/40" />}
      </div>
      {/* laptop base */}
      <div
        className={cn(
          "h-[6px] w-[64px] rounded-b border-x-2 border-b-2",
          active ? "border-primary" : "border-muted-foreground",
        )}
      />
    </div>
  );
}

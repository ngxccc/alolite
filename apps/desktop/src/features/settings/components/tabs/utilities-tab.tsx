import { useEffect, useState } from "react";
import { ipc, isTauriRuntime } from "@/shared/api/ipc";
import { useUIStore } from "@/stores";
import type { AutoReplyRule } from "@/shared/types/ipc.generated";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SectionHeading, SettingCard, SwitchRow } from "../primitives";
import type { SettingsTabProps } from "../types";

export function UtilitiesTab({ settings, updateSettings }: SettingsTabProps) {
  const showToast = useUIStore((s) => s.showToast);
  const [autoReplyRules, setAutoReplyRules] = useState<AutoReplyRule[]>([]);
  const [newRuleKw, setNewRuleKw] = useState("");
  const [newRuleReply, setNewRuleReply] = useState("");

  useEffect(() => {
    if (!isTauriRuntime()) return;
    void ipc
      .getAutoreplyRules()
      .then((r) => setAutoReplyRules(Array.isArray(r) ? r : []))
      .catch(() => undefined);
  }, []);

  return (
    <>
      <SectionHeading>Gợi ý nhãn dán</SectionHeading>
      <SettingCard>
        <SwitchRow
          label="Hiển thị gợi ý sticker theo từ khóa đang soạn"
          checked={settings.suggest_stickers_while_typing}
          onChange={(v) => updateSettings({ suggest_stickers_while_typing: v })}
        />
      </SettingCard>

      <SectionHeading>Đa phiên bản</SectionHeading>
      <SettingCard>
        <div className="px-4 py-3">
          <div className="flex gap-1.5">
            <Input
              value={settings.multi_profile_name}
              onChange={(e) => updateSettings({ multi_profile_name: e.target.value })}
              placeholder="Tên profile phụ (vd. acc2)"
            />
            <Button
              variant="outline"
              onClick={() => void ipc.spawnMultiProfile(settings.multi_profile_name)}
            >
              Mở Profile mới
            </Button>
          </div>
        </div>
      </SettingCard>

      <SectionHeading>Tự động trả lời</SectionHeading>
      <SettingCard>
        <div className="px-4 py-3">
          {autoReplyRules.map((rule) => (
            <div
              key={rule.id}
              className="flex items-center justify-between border-b border-border py-2 last:border-0"
            >
              <span className="text-sm">
                <code className="rounded bg-muted px-1.5 py-0.5 text-xs">{rule.keyword}</code> →{" "}
                {rule.reply_text}
              </span>
              <Button
                variant="destructive"
                size="sm"
                onClick={async () => {
                  const next = autoReplyRules.filter((r) => r.id !== rule.id);
                  setAutoReplyRules(next);
                  await ipc.saveAutoreplyRules(next);
                }}
              >
                Xóa
              </Button>
            </div>
          ))}
          <div className="mt-2 flex gap-1.5">
            <Input
              value={newRuleKw}
              onChange={(e) => setNewRuleKw(e.target.value)}
              placeholder="Từ khóa (vd. /ping)"
            />
            <Input
              value={newRuleReply}
              onChange={(e) => setNewRuleReply(e.target.value)}
              placeholder="Nội dung trả lời…"
            />
            <Button
              variant="default"
              onClick={async () => {
                if (!newRuleKw.trim() || !newRuleReply.trim()) return;
                const next: AutoReplyRule[] = [
                  ...autoReplyRules,
                  {
                    id: `rule_${Date.now()}`,
                    keyword: newRuleKw.trim(),
                    match_mode: "contains",
                    reply_text: newRuleReply.trim(),
                    enabled: true,
                    scope: "direct",
                  },
                ];
                setAutoReplyRules(next);
                setNewRuleKw("");
                setNewRuleReply("");
                await ipc.saveAutoreplyRules(next);
              }}
            >
              Thêm
            </Button>
          </div>
        </div>
      </SettingCard>

      <SectionHeading>Nhãn dán tùy chỉnh</SectionHeading>
      <SettingCard>
        <div className="px-4 py-3">
          <Button
            variant="outline"
            size="sm"
            onClick={async () => {
              const p = await ipc.exportCustomStickersTemplate();
              showToast(`Đã xuất mẫu ${p}`);
            }}
          >
            Xuất mẫu custom_stickers.json
          </Button>
        </div>
      </SettingCard>
    </>
  );
}

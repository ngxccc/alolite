import { RowDivider, SectionHeading, SelectRow, SettingCard, SwitchRow } from "../primitives";
import type { SettingsTabProps } from "../types";

export function MessagesTab({ settings, updateSettings }: SettingsTabProps) {
  return (
    <>
      <SectionHeading>Cài đặt tin nhắn</SectionHeading>
      <SettingCard>
        <SwitchRow
          label="Nhấp đúp vào tin nhắn để trả lời"
          checked={settings.double_click_to_reply}
          onChange={(v) => updateSettings({ double_click_to_reply: v })}
        />
        <RowDivider />
        <SwitchRow
          label="Hiển thị trạng thái đang soạn tin"
          checked={settings.show_typing_indicator}
          onChange={(v) => updateSettings({ show_typing_indicator: v })}
        />
        <RowDivider />
        <SelectRow
          label="Phím gửi tin nhắn"
          value={settings.send_message_key}
          onChange={(v) =>
            updateSettings({
              send_message_key: v,
            })
          }
          options={[
            { value: "enter", label: "Enter để gửi" },
            { value: "cmd_enter", label: "Ctrl/Cmd + Enter để gửi" },
          ]}
        />
      </SettingCard>
    </>
  );
}

import { RowDivider, SectionHeading, SettingCard, SwitchRow } from "../primitives";
import type { SettingsTabProps } from "../types";

export function PrivacyTab({ settings, updateSettings }: SettingsTabProps) {
  return (
    <>
      <SectionHeading>Trạng thái hoạt động</SectionHeading>
      <SettingCard>
        <SwitchRow
          label="Hiển thị trạng thái truy cập"
          checked={settings.show_online_status}
          onChange={(v) => updateSettings({ show_online_status: v })}
        />
      </SettingCard>

      <SectionHeading>Tin nhắn</SectionHeading>
      <SettingCard>
        <SwitchRow
          label='Hiển thị trạng thái "Đã xem"'
          checked={settings.show_seen_status}
          onChange={(v) => updateSettings({ show_seen_status: v, block_seen: !v })}
        />
        <RowDivider />
        <SwitchRow
          label='Ẩn trạng thái "Đang soạn tin"'
          desc="Chặn gửi gói typing đến người khác."
          checked={settings.block_typing}
          onChange={(v) => updateSettings({ block_typing: v })}
        />
        <RowDivider />
        <SwitchRow
          label='Ẩn trạng thái "Đã nhận"'
          desc="Chặn gửi xác nhận đã nhận tin."
          checked={settings.block_delivered}
          onChange={(v) => updateSettings({ block_delivered: v })}
        />
      </SettingCard>
    </>
  );
}

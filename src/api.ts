// The only module that talks to Tauri. No call here carries a frame, an address or a function code:
// control requests name a device, a profile command or register, and an engineering value; the Rust
// command service checks arming, bounds and confirmation and builds the frame (safety rules 2-3).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AuditRow,
  CommandOutcome,
  CommSettingKind,
  ControlStatus,
  Preview,
  ReportData,
  Role,
  Wizard,
  WizardSupport,
  AlarmDef,
  AlarmSnapshot,
  AppInfo,
  DeviceInspection,
  EventRow,
  ExportFormat,
  ExportResult,
  LiveFrame,
  LiveSnapshot,
  PortView,
  ProfileView,
  TrainingFault,
  TrainingStatus,
  TrendSeries,
} from "./types.ts";

export const FRAME_EVENT = "buslink://frame";

export function hasBackend(): boolean {
  return typeof (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ !== "undefined";
}

/** Error text from a rejected command (Rust returns plain strings). */
export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

export interface ConnectReq {
  kind: "rtu" | "tcp" | "rtu_over_tcp";
  port?: string;
  baud?: number;
  data_bits?: number;
  parity?: "none" | "even" | "odd";
  stop_bits?: number;
  host?: string;
  tcp_port?: number;
}

export interface ConnectResult {
  link_id: string;
  isolation_advice: boolean;
  otg_battery_advice: boolean;
  high_baud_warning: boolean;
}

/** What Start polling achieved beyond polling (Android foreground service). */
export interface StartResult {
  android: boolean;
  background: boolean;
  notifications_denied: boolean;
  error: string | null;
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listPorts: () => invoke<PortView[]>("list_ports"),
  listProfiles: () => invoke<ProfileView[]>("list_profiles"),
  connect: (req: ConnectReq) => invoke<ConnectResult>("connect_link", { req }),
  reconnect: (id: string) => invoke<void>("reconnect_link", { id }),
  disconnect: (id: string) => invoke<void>("disconnect_link", { id }),
  removeLink: (id: string) => invoke<void>("remove_link", { id }),
  startPolling: (id: string) => invoke<StartResult>("start_polling", { id }),
  stopPolling: (id: string) => invoke<void>("stop_polling", { id }),
  addDevice: (req: { link_id: string; profile_key: string; unit_id: number; name?: string; poll_interval_ms?: number }) =>
    invoke<string>("add_device", { req }),
  removeDevice: (id: string) => invoke<void>("remove_device", { id }),
  snapshot: () => invoke<LiveSnapshot>("live_snapshot"),
  trend: (req: { device_id: string; tag_id: string; from_ms: number; to_ms: number; buckets: number }) =>
    invoke<TrendSeries>("trend_query", { req }),
  exportSession: (format: ExportFormat) => invoke<ExportResult>("export_session", { format }),
  inspect: (id: string) => invoke<DeviceInspection>("inspect_device", { id }),
  alarms: () => invoke<AlarmSnapshot>("alarm_list"),
  alarmAdd: (def: AlarmDef) => invoke<void>("alarm_add", { def }),
  alarmRemove: (id: string) => invoke<void>("alarm_remove", { id }),
  alarmAck: (id: string) => invoke<void>("alarm_ack", { id }),
  alarmShelve: (id: string, minutes: number) => invoke<void>("alarm_shelve", { req: { id, minutes } }),
  alarmUnshelve: (id: string) => invoke<void>("alarm_unshelve", { id }),
  alarmSetOos: (id: string, out_of_service: boolean) => invoke<void>("alarm_set_oos", { req: { id, out_of_service } }),
  alarmEvents: () => invoke<EventRow[]>("alarm_events"),
  alarmDefaults: (unit: string | null, span: number) =>
    invoke<{ deadband: number; delay_ms: number }>("alarm_defaults", { req: { unit, span } }),
  trainingStatus: () => invoke<TrainingStatus>("training_status"),
  trainingStart: () => invoke<TrainingStatus>("training_start"),
  trainingFault: (unit: number, fault: TrainingFault, seconds: number) =>
    invoke<TrainingStatus>("training_fault", { req: { unit, fault, seconds } }),
  trainingClear: (unit: number) => invoke<TrainingStatus>("training_clear", { unit }),
  // ---- control (simulated devices) and commissioning ----
  controlStatus: () => invoke<ControlStatus>("control_status"),
  controlArm: (device_id: string, user: string, role: Role) => invoke<ControlStatus>("control_arm", { req: { device_id, user, role } }),
  controlDisarm: (reason: "operator" | "app_background") => invoke<ControlStatus>("control_disarm", { reason }),
  controlPreview: (device_id: string, request: PanelRequest) => invoke<Preview>("control_preview", { req: { device_id, request } }),
  controlConfirm: (preview_id: number) => invoke<ControlStatus>("control_confirm", { previewId: preview_id }),
  controlExecute: () => invoke<CommandOutcome>("control_execute"),
  controlCancel: () => invoke<ControlStatus>("control_cancel"),
  wizardSupport: (device_id: string) => invoke<WizardSupport>("wizard_support", { deviceId: device_id }),
  wizardStart: (device_id: string, setting: CommSettingKind, value: number, user: string) =>
    invoke<Wizard>("wizard_start", { req: { device_id, setting, value, user } }),
  wizardPreview: () => invoke<Preview>("wizard_preview"),
  wizardPowerCycled: () => invoke<Wizard>("wizard_power_cycled"),
  wizardReconnect: () => invoke<Wizard>("wizard_reconnect"),
  wizardVerify: () => invoke<Wizard>("wizard_verify"),
  wizardRevert: () => invoke<Wizard>("wizard_revert"),
  wizardClose: () => invoke<ControlStatus>("wizard_close"),
  auditList: () => invoke<AuditRow[]>("audit_list"),
  auditExport: (format: "csv" | "xlsx") => invoke<ExportResult>("audit_export", { format }),
  reportData: () => invoke<ReportData>("report_data"),
  reportSaveHtml: (html: string) => invoke<ExportResult>("report_save_html", { html }),
};

/** A control panel request: a profile command, or a read_write register by ID, in engineering units. */
export type PanelRequest =
  | { kind: "named"; command: string; value?: number }
  | { kind: "register"; register: string; value: number };

export function onFrame(cb: (f: LiveFrame) => void): Promise<UnlistenFn> {
  return listen<LiveFrame>(FRAME_EVENT, (e) => cb(e.payload));
}

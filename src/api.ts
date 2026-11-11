// The only module that talks to Tauri. Monitoring only: there is no write call here.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
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

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  listPorts: () => invoke<PortView[]>("list_ports"),
  listProfiles: () => invoke<ProfileView[]>("list_profiles"),
  connect: (req: ConnectReq) => invoke<{ link_id: string; isolation_advice: boolean }>("connect_link", { req }),
  reconnect: (id: string) => invoke<void>("reconnect_link", { id }),
  disconnect: (id: string) => invoke<void>("disconnect_link", { id }),
  removeLink: (id: string) => invoke<void>("remove_link", { id }),
  startPolling: (id: string) => invoke<void>("start_polling", { id }),
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
};

export function onFrame(cb: (f: LiveFrame) => void): Promise<UnlistenFn> {
  return listen<LiveFrame>(FRAME_EVENT, (e) => cb(e.payload));
}

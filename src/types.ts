// Wire types mirrored from the Rust session service (crates/buslink-core/src/session).

export type QualityCode = "GOOD" | "TIMEOUT" | "CRC_ERROR" | "EXCEPTION" | "STALE" | "DECODE_ERROR" | "DISCONNECTED";

export interface TagView {
  key: string;
  link_id: string;
  device_id: string;
  tag_id: string;
  quality: QualityCode;
  exception: number | null;
  value: number | null;
  text: string | null;
  raw_value: string | null;
  value_ts_ms: number | null;
  ts_ms: number;
  age_ms: number | null;
  raw: number[] | null;
  unit: string | null;
  error: string | null;
  seq: number;
}

export interface TrendPoint {
  k: string;
  t: number;
  v: number | null;
  q: QualityCode;
}

export interface DeviceCounters {
  requests: number;
  good: number;
  timeouts: number;
  crc_errors: number;
  exceptions: number;
  decode_errors: number;
  last_error: string | null;
  cycle_ms: number | null;
  consecutive_failures: number;
  backoff_until_mono_ms: number | null;
}

export interface DeviceView {
  id: string;
  name: string;
  link_id: string;
  unit_id: number;
  profile_id: string;
  profile_version: string;
  simulated_only: boolean;
  poll_interval_ms: number;
  stale_after_ms: number;
  counters: DeviceCounters;
}

export type LinkKind = "rtu" | "tcp" | "rtu_over_tcp" | "simulator";
export type LinkStateCode = "connected" | "polling" | "down" | "disconnected";

export interface LinkView {
  id: string;
  kind: LinkKind;
  label: string;
  simulated: boolean;
  state: LinkStateCode;
  error: string | null;
  devices: DeviceView[];
}

export type AlarmKind = "HI" | "HI_HI" | "LO" | "LO_LO";
export type AlarmStateCode =
  | "NORMAL"
  | "UNACKED"
  | "ACKED"
  | "RTN_UNACKED"
  | "SHELVED"
  | "SUPPRESSED"
  | "OUT_OF_SERVICE";

export interface AlarmView {
  id: string;
  class: "process" | "comm";
  device_id: string;
  tag_id: string | null;
  kind: AlarmKind | null;
  limit: number | null;
  deadband: number | null;
  on_delay_ms: number;
  off_delay_ms: number;
  priority: number;
  latch: boolean;
  message: string | null;
  state: AlarmStateCode;
  active: boolean;
  acked: boolean;
  held: boolean;
  shelved_until_ms: number | null;
  out_of_service: boolean;
  chattering: boolean;
  activated_ms: number | null;
  last_change_ms: number;
  value: number | null;
}

export interface AlarmSummary {
  counts: [number, number, number];
  unacked: number;
  oldest_unacked: string | null;
  oldest_unacked_ms: number | null;
  activations_10min: number;
  flood: boolean;
  comm_active: number;
}

export interface AlarmSnapshot {
  alarms: AlarmView[];
  summary: AlarmSummary;
}

export interface AlarmDef {
  id: string;
  device_id: string;
  tag_id: string;
  kind: AlarmKind;
  limit: number;
  deadband: number;
  on_delay_ms: number;
  off_delay_ms: number;
  priority: number;
  latch: boolean;
  message: string | null;
}

export interface LiveFrame {
  ts_ms: number;
  tags: TagView[];
  points: TrendPoint[];
  dropped: number;
  links: LinkView[] | null;
  alarms: AlarmSnapshot | null;
  historian_error: string | null;
}

export interface LiveSnapshot {
  ts_ms: number;
  session_id: string;
  tags: TagView[];
  links: LinkView[];
  alarms: AlarmSnapshot;
  historian_error: string | null;
}

export type Text = string | { en: string; id: string };
export type SupportLevel = "generic" | "documented" | "simulated" | "hardware_verified";

export interface Register {
  id: string;
  name: Text;
  description?: Text;
  table: "coil" | "discrete_input" | "input_register" | "holding_register";
  address: number;
  doc_address?: string | number;
  type: string;
  length?: number;
  word_order?: string;
  scale?: number;
  offset?: number;
  unit?: string;
  decimals?: number;
  valid_range?: [number, number];
  bits?: Record<string, Text>;
  access?: "read" | "read_write";
}

export interface Profile {
  profile_id: string;
  profile_version: string;
  vendor: Text;
  model: Text;
  category?: Text;
  description?: Text;
  support_level: SupportLevel;
  simulated_only: boolean;
  registers: Register[];
  transport: {
    rtu?: { unit_id: number; baud: number; data_bits: number; parity: string; stop_bits: number };
    tcp?: { unit_id: number; port: number };
    simulator?: { model: string };
  };
  source?: { document: Text; revision: Text };
}

export interface ProfileView {
  key: string;
  profile: Profile;
}

export interface UsbChip {
  chip: string;
  vendor: string;
  driver: string;
}

export interface PortView {
  name: string;
  kind: "usb" | "pci" | "bluetooth" | "unknown";
  vid: number | null;
  pid: number | null;
  vid_pid: string | null;
  chip: UsbChip | null;
  manufacturer: string | null;
  product: string | null;
  serial_number: string | null;
}

export interface Gap {
  from_ms: number;
  to_ms: number;
  reason: string;
}

export interface TrendSeries {
  t: number[];
  v: (number | null)[];
  gaps: Gap[];
  decimated: boolean;
  raw_count: number;
}

export interface OrderValue {
  order: string;
  value: string | null;
  error: string | null;
  is_profile: boolean;
}

export interface InspectRow {
  tag_id: string;
  name: Text;
  table: string;
  doc_address: string;
  pdu_offset: number;
  function: number;
  function_name: string;
  type_name: string;
  word_order: string | null;
  width: number;
  unit: string | null;
  raw: number[] | null;
  raw_hex: string | null;
  decoded: string | null;
  value: number | null;
  orders: OrderValue[];
  quality: QualityCode | null;
  exception: number | null;
  exception_name: string | null;
  error: string | null;
  value_ts_ms: number | null;
}

export interface FrameEntry {
  ts_ms: number;
  unit: number;
  request_hex: string;
  response_hex: string;
  outcome: string;
  error: string | null;
  elapsed_ms: number;
  discarded: number;
}

export interface DeviceInspection {
  device: DeviceView;
  rows: InspectRow[];
  frames: FrameEntry[];
}

export interface EventRow {
  session: string | null;
  ts_ms: number;
  kind: string;
  device: string | null;
  tag: string | null;
  severity: string;
  text: string;
}

export type TrainingFault = "timeout" | "crc_error" | "exception02" | "stale" | "wrong_order";

export interface ActiveFault {
  id: number;
  unit: number;
  fault: TrainingFault;
  until_mono_ms: number;
  remaining_ms: number;
}

export interface TrainingStatus {
  started: boolean;
  link_id: string;
  units: number[];
  faults: ActiveFault[];
}

export interface AppInfo {
  name: string;
  version: string;
  platform: string;
  session_id: string;
  historian_path: string | null;
  profile_errors: string[];
}

export interface ExportResult {
  saved: boolean;
  path: string | null;
  bytes: number;
}

export type ExportFormat = "csv" | "xlsx" | "json";

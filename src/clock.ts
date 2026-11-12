// Time source for ages and ranges. On the desktop it is the local clock. In the LAN
// browser view the values were stamped by the host PC, so ages and trend ranges use the
// host's time: the browser clock may be minutes off and must not turn good data STALE.
let offsetMs = 0;

export function nowMs(): number {
  return Date.now() + offsetMs;
}

/** Align with the host: called with the `ts_ms` of every snapshot and frame. */
export function syncServerTime(serverMs: number, localMs: number = Date.now()): void {
  offsetMs = serverMs - localMs;
}

export function clockOffsetMs(): number {
  return offsetMs;
}

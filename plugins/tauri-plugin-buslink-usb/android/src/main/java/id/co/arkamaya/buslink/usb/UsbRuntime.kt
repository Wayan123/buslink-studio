package id.co.arkamaya.buslink.usb

import java.util.concurrent.ConcurrentHashMap

/**
 * Process-wide USB state shared by the plugin and the foreground service.
 * Nothing here persists: a new process starts with no open port and no service
 * (reattach or restart never resumes polling, safety rule 6).
 */
object UsbRuntime {
    /** Open ports by port ID. One owner per port: a second open is refused. */
    val sessions = ConcurrentHashMap<String, SerialSession>()

    @Volatile var lastEvent: String? = null
    @Volatile var serviceRunning: Boolean = false
    @Volatile var serviceError: String? = null

    fun event(text: String) {
        lastEvent = text
    }

    fun closeAll(reason: String) {
        val all = sessions.values.toList()
        sessions.clear()
        for (s in all) s.close(EndState.CLOSED)
        if (all.isNotEmpty()) event(reason)
    }

    /** The adapter with this device node was unplugged. */
    fun detached(deviceName: String) {
        val hit = sessions.values.filter { it.deviceName == deviceName }
        for (s in hit) {
            sessions.remove(s.portId)
            s.close(EndState.DETACHED)
        }
        if (hit.isNotEmpty()) {
            event("USB adapter detached: $deviceName")
            PollingService.instance?.stopSelf()
        }
    }
}

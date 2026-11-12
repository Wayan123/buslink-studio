package id.co.arkamaya.buslink.usb

import android.os.Process
import com.hoho.android.usbserial.driver.UsbSerialPort
import java.io.IOException

/**
 * One open USB serial port with its reader thread.
 * Reads use short timeouts; every packet goes to [queue] for the Rust bridge,
 * which frames RTU replies by expected length. No hardware purge is ever
 * called (the CH340 driver throws on it): Rust drains by reading until quiet.
 */
class SerialSession(
    val portId: String,
    val deviceName: String,
    private val port: UsbSerialPort,
) {
    val queue = PacketQueue()
    private val writeLock = Any()
    @Volatile private var closing = false
    private val reader = Thread({ readLoop() }, "buslink-usb-read")

    fun start() {
        reader.isDaemon = true
        reader.start()
    }

    private fun readLoop() {
        // Android is not real-time; a favourable priority reduces lost fragments at higher baud rates.
        try {
            Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_DISPLAY)
        } catch (_: Exception) {
        }
        val buf = ByteArray(1024)
        while (!closing) {
            try {
                val n = port.read(buf, READ_TIMEOUT_MS)
                if (n > 0) queue.offer(buf.copyOf(n))
            } catch (e: Exception) {
                if (!closing) {
                    UsbRuntime.event("USB read failed on $portId: ${e.message}")
                    UsbRuntime.sessions.remove(portId, this)
                    queue.end(EndState.DETACHED)
                    try {
                        port.close()
                    } catch (_: Exception) {
                    }
                }
                return
            }
        }
    }

    /** Throws [IOException] when the port is gone. */
    fun write(bytes: ByteArray) {
        if (closing) throw IOException("port closed")
        synchronized(writeLock) { port.write(bytes, WRITE_TIMEOUT_MS) }
    }

    fun close(state: EndState) {
        if (closing) return
        closing = true
        queue.end(state)
        try {
            port.close()
        } catch (_: Exception) {
        }
        reader.interrupt()
    }

    companion object {
        const val READ_TIMEOUT_MS = 50
        const val WRITE_TIMEOUT_MS = 1000
    }
}

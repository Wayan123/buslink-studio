package id.co.arkamaya.buslink.usb

import android.Manifest
import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.DocumentsContract
import android.webkit.WebView
import androidx.activity.result.ActivityResult
import androidx.appcompat.app.AppCompatActivity
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.hoho.android.usbserial.driver.UsbSerialPort
import com.hoho.android.usbserial.driver.UsbSerialProber
import java.io.File
import java.io.IOException
import java.util.concurrent.Executors

@InvokeArg
class DeviceArgs {
    lateinit var deviceName: String
}

@InvokeArg
class OpenArgs {
    lateinit var deviceName: String
    var portIndex: Int = 0
    var baud: Int = 9600
    var dataBits: Int = 8
    var parity: String = "none"
    var stopBits: Int = 1
}

@InvokeArg
class ReadArgs {
    lateinit var portId: String
    var maxBytes: Int = 256
    var timeoutMs: Long = 50
}

@InvokeArg
class WriteArgs {
    lateinit var portId: String
    lateinit var data: String
}

@InvokeArg
class PortArgs {
    lateinit var portId: String
}

@InvokeArg
class ServiceArgs {
    var label: String = ""
    var state: String = "stopped"
    var devices: Int = 0
}

@InvokeArg
class SaveArgs {
    lateinit var cacheFileName: String
    lateinit var displayName: String
    lateinit var mime: String
}

/**
 * Android half of the BusLink USB bridge. Only the Rust side of the app calls
 * these commands (no webview capability is granted). There is no write-frame
 * logic here: `writePort` forwards bytes that the Rust core already encoded.
 */
@TauriPlugin(
    permissions = [Permission(strings = [Manifest.permission.POST_NOTIFICATIONS], alias = "notifications")],
)
class BusLinkUsbPlugin(private val activity: Activity) : Plugin(activity) {
    private val io = Executors.newCachedThreadPool { r -> Thread(r, "buslink-usb-io").also { it.isDaemon = true } }
    private val main = Handler(Looper.getMainLooper())
    private val usb get() = activity.getSystemService(Context.USB_SERVICE) as UsbManager
    private val lock = Any()

    private class Pending(val invoke: Invoke, val deviceName: String, val requestId: Int) {
        var receiver: BroadcastReceiver? = null
        var timeout: Runnable? = null
    }

    private var pending: Pending? = null
    private var nextRequestId = 1

    private class PendingSave(val cacheFile: File, val mime: String)

    private var pendingSave: PendingSave? = null

    private var detachReceiver: BroadcastReceiver? = null

    // ---- lifecycle -------------------------------------------------------

    override fun load(webView: WebView) {
        val r = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                if (intent.action != UsbManager.ACTION_USB_DEVICE_DETACHED) return
                val device = deviceExtra(intent) ?: return
                UsbRuntime.detached(device.deviceName)
                // A permission request for a device that left must not open anything later.
                synchronized(lock) { pending?.takeIf { it.deviceName == device.deviceName } }?.let {
                    finishPermission(it, "unavailable", "The adapter was unplugged.")
                }
            }
        }
        detachReceiver = r
        ContextCompat.registerReceiver(
            activity, r, IntentFilter(UsbManager.ACTION_USB_DEVICE_DETACHED), ContextCompat.RECEIVER_NOT_EXPORTED,
        )
    }

    override fun onStop(activity: AppCompatActivity) {
        synchronized(lock) { pending }?.let { finishPermission(it, "unavailable", "The app left the foreground.") }
    }

    override fun onDestroy(activity: AppCompatActivity) {
        synchronized(lock) { pending }?.let { finishPermission(it, "unavailable", "The app was closed.") }
        detachReceiver?.let {
            try {
                activity.unregisterReceiver(it)
            } catch (_: Exception) {
            }
        }
        detachReceiver = null
        // Activity gone for good: stop the service and release the ports. A mere
        // recreation (rotation) keeps them: only finishing counts.
        if (activity.isFinishing) {
            UsbRuntime.closeAll("App closed")
            PollingService.instance?.stopSelf()
        }
    }

    private fun inForeground(): Boolean {
        val a = activity as? AppCompatActivity ?: return true
        return a.lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
    }

    @Suppress("DEPRECATION")
    private fun deviceExtra(intent: Intent): UsbDevice? =
        if (Build.VERSION.SDK_INT >= 33) {
            intent.getParcelableExtra(UsbManager.EXTRA_DEVICE, UsbDevice::class.java)
        } else {
            intent.getParcelableExtra(UsbManager.EXTRA_DEVICE)
        }

    // ---- enumeration -----------------------------------------------------

    @Command
    fun listDevices(invoke: Invoke) {
        io.execute {
            try {
                val ports = JSArray()
                for (driver in UsbSerialProber.getDefaultProber().findAllDrivers(usb)) {
                    val d = driver.device
                    for ((index, _) in driver.ports.withIndex()) {
                        val o = JSObject()
                        o.put("deviceName", d.deviceName)
                        o.put("portIndex", index)
                        o.put("vid", d.vendorId)
                        o.put("pid", d.productId)
                        o.put("manufacturer", safe { d.manufacturerName })
                        o.put("product", safe { d.productName })
                        o.put("serialNumber", safe { d.serialNumber })
                        o.put("driver", driver.javaClass.simpleName)
                        o.put("hasPermission", usb.hasPermission(d))
                        ports.put(o)
                    }
                }
                val res = JSObject()
                res.put("ports", ports)
                invoke.resolve(res)
            } catch (e: Exception) {
                invoke.reject("Could not list USB devices: ${e.message}")
            }
        }
    }

    private inline fun <T> safe(block: () -> T): T? =
        try {
            block()
        } catch (_: Exception) {
            null // serial number needs permission on Android 10+
        }

    // ---- permission ------------------------------------------------------

    @Command
    fun requestPermission(invoke: Invoke) {
        val args = invoke.parseArgs(DeviceArgs::class.java)
        val device = usb.deviceList[args.deviceName]
        if (device == null) {
            resolvePermission(invoke, "unavailable", "The adapter is no longer connected.")
            return
        }
        if (usb.hasPermission(device)) {
            resolvePermission(invoke, "granted", null)
            return
        }
        if (!inForeground()) {
            resolvePermission(invoke, "unavailable", "USB permission can only be requested while the app is open.")
            return
        }
        val req: Pending
        synchronized(lock) {
            if (pending != null) {
                resolvePermission(invoke, "busy", "Another USB permission request is open.")
                return
            }
            req = Pending(invoke, args.deviceName, nextRequestId++)
            pending = req
        }
        main.post {
            val action = "${activity.packageName}.USB_PERMISSION"
            val receiver = object : BroadcastReceiver() {
                override fun onReceive(context: Context, intent: Intent) {
                    if (intent.action != action) return
                    if (intent.getIntExtra(EXTRA_REQUEST_ID, -1) != req.requestId) return
                    val dev = deviceExtra(intent)
                    if (dev != null && dev.deviceName != req.deviceName) return
                    val granted = intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)
                    if (granted) {
                        finishPermission(req, "granted", null)
                    } else {
                        finishPermission(
                            req,
                            "denied",
                            "USB permission was denied. Unplug and replug the adapter, tap Allow, and tick " +
                                "\"Use by default for this USB device\".",
                        )
                    }
                }
            }
            req.receiver = receiver
            ContextCompat.registerReceiver(activity, receiver, IntentFilter(action), ContextCompat.RECEIVER_NOT_EXPORTED)
            val timeout = Runnable { finishPermission(req, "timed_out", "No answer to the USB permission prompt.") }
            req.timeout = timeout
            main.postDelayed(timeout, PERMISSION_TIMEOUT_MS)
            // Explicit (package-scoped) intent: required for mutable PendingIntents on API 34+.
            val intent = Intent(action).setPackage(activity.packageName).putExtra(EXTRA_REQUEST_ID, req.requestId)
            val pi = PendingIntent.getBroadcast(
                activity,
                req.requestId,
                intent,
                PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_CANCEL_CURRENT,
            )
            try {
                usb.requestPermission(device, pi)
            } catch (e: Exception) {
                finishPermission(req, "unavailable", "Could not ask for USB permission: ${e.message}")
            }
        }
    }

    private fun finishPermission(req: Pending, state: String, message: String?) {
        synchronized(lock) {
            if (pending !== req) return
            pending = null
        }
        req.timeout?.let { main.removeCallbacks(it) }
        req.receiver?.let {
            try {
                activity.unregisterReceiver(it)
            } catch (_: Exception) {
            }
        }
        resolvePermission(req.invoke, state, message)
    }

    private fun resolvePermission(invoke: Invoke, state: String, message: String?) {
        val o = JSObject()
        o.put("state", state)
        if (message != null) o.put("message", message)
        invoke.resolve(o)
    }

    // ---- port I/O --------------------------------------------------------

    @Command
    fun openPort(invoke: Invoke) {
        val args = invoke.parseArgs(OpenArgs::class.java)
        io.execute {
            try {
                if (args.baud < 300 || args.baud > 3_000_000) return@execute invoke.reject("baud must be 300..3000000")
                if (args.dataBits != 7 && args.dataBits != 8) return@execute invoke.reject("data bits must be 7 or 8")
                if (args.stopBits != 1 && args.stopBits != 2) return@execute invoke.reject("stop bits must be 1 or 2")
                val parity = when (args.parity) {
                    "none" -> UsbSerialPort.PARITY_NONE
                    "even" -> UsbSerialPort.PARITY_EVEN
                    "odd" -> UsbSerialPort.PARITY_ODD
                    else -> return@execute invoke.reject("parity must be none, even or odd")
                }
                val portId = "${args.deviceName}#${args.portIndex}"
                if (UsbRuntime.sessions.containsKey(portId)) {
                    return@execute invoke.reject("$portId is already open (one master per bus)")
                }
                val device = usb.deviceList[args.deviceName]
                    ?: return@execute invoke.reject("disconnected: the adapter is not connected")
                if (!usb.hasPermission(device)) return@execute invoke.reject("permission: USB permission is missing")
                val driver = UsbSerialProber.getDefaultProber().probeDevice(device)
                    ?: return@execute invoke.reject("No serial driver for ${"%04X:%04X".format(device.vendorId, device.productId)}")
                val port = driver.ports.getOrNull(args.portIndex)
                    ?: return@execute invoke.reject("The adapter has no port ${args.portIndex}")
                val connection = usb.openDevice(device)
                    ?: return@execute invoke.reject("permission: could not open the USB device")
                try {
                    port.open(connection)
                    port.setParameters(
                        args.baud,
                        args.dataBits,
                        if (args.stopBits == 2) UsbSerialPort.STOPBITS_2 else UsbSerialPort.STOPBITS_1,
                        parity,
                    )
                } catch (e: Exception) {
                    try {
                        port.close()
                    } catch (_: Exception) {
                    }
                    return@execute invoke.reject("Could not open ${args.deviceName}: ${e.message}")
                }
                val session = SerialSession(portId, args.deviceName, port)
                if (UsbRuntime.sessions.putIfAbsent(portId, session) != null) {
                    session.close(EndState.CLOSED)
                    return@execute invoke.reject("$portId is already open (one master per bus)")
                }
                session.start()
                UsbRuntime.event("Opened $portId at ${args.baud} baud")
                val res = JSObject()
                res.put("portId", portId)
                invoke.resolve(res)
            } catch (e: Exception) {
                invoke.reject("Could not open the port: ${e.message}")
            }
        }
    }

    @Command
    fun readPort(invoke: Invoke) {
        val args = invoke.parseArgs(ReadArgs::class.java)
        io.execute {
            val res = JSObject()
            val s = UsbRuntime.sessions[args.portId]
            if (s == null) {
                res.put("data", "")
                res.put("state", "closed")
                return@execute invoke.resolve(res)
            }
            val timeout = args.timeoutMs.coerceIn(0, 5000)
            when (val r = s.queue.poll(timeout, args.maxBytes.coerceIn(1, 1024))) {
                is PollResult.Data -> {
                    res.put("data", Hex.encode(r.bytes))
                    res.put("state", "ok")
                }
                is PollResult.Timeout -> {
                    res.put("data", "")
                    res.put("state", "ok")
                }
                is PollResult.Ended -> {
                    res.put("data", "")
                    res.put("state", r.state.wire)
                }
            }
            invoke.resolve(res)
        }
    }

    @Command
    fun writePort(invoke: Invoke) {
        val args = invoke.parseArgs(WriteArgs::class.java)
        io.execute {
            val s = UsbRuntime.sessions[args.portId]
                ?: return@execute invoke.reject("disconnected: port is closed")
            val bytes = Hex.decode(args.data)
            if (bytes == null || bytes.isEmpty() || bytes.size > 512) {
                return@execute invoke.reject("write data must be 1..512 bytes of hex")
            }
            try {
                s.write(bytes)
                invoke.resolve()
            } catch (e: IOException) {
                UsbRuntime.sessions.remove(args.portId, s)
                s.close(EndState.DETACHED)
                invoke.reject("disconnected: ${e.message}")
            } catch (e: Exception) {
                invoke.reject("write failed: ${e.message}")
            }
        }
    }

    @Command
    fun closePort(invoke: Invoke) {
        val args = invoke.parseArgs(PortArgs::class.java)
        io.execute {
            UsbRuntime.sessions.remove(args.portId)?.close(EndState.CLOSED)
            invoke.resolve()
        }
    }

    // ---- notifications and foreground service ----------------------------

    private fun notificationsGranted(): Boolean {
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            return false
        }
        return NotificationManagerCompat.from(activity).areNotificationsEnabled()
    }

    private fun notificationState() = JSObject().also { it.put("notifications", if (notificationsGranted()) "granted" else "denied") }

    @Command
    fun requestNotificationPermission(invoke: Invoke) {
        if (Build.VERSION.SDK_INT < 33 || notificationsGranted()) {
            invoke.resolve(notificationState())
        } else {
            requestPermissionForAlias("notifications", invoke, "notificationResult")
        }
    }

    @PermissionCallback
    fun notificationResult(invoke: Invoke) {
        invoke.resolve(notificationState())
    }

    @Command
    fun startService(invoke: Invoke) {
        val args = invoke.parseArgs(ServiceArgs::class.java)
        if (!inForeground()) {
            // Android 12+ forbids starting a foreground service from the background.
            return invoke.reject("Start polling while the app is open (foreground service rule).")
        }
        io.execute {
            try {
                UsbRuntime.serviceError = null
                val svc = PollingService.instance
                if (svc != null) {
                    svc.update(args.label, args.state, args.devices)
                } else {
                    val intent = Intent(activity, PollingService::class.java)
                        .putExtra(PollingService.EXTRA_LABEL, args.label)
                        .putExtra(PollingService.EXTRA_STATE, args.state)
                        .putExtra(PollingService.EXTRA_DEVICES, args.devices)
                    ContextCompat.startForegroundService(activity, intent)
                    val until = System.currentTimeMillis() + 3000
                    while (!UsbRuntime.serviceRunning && UsbRuntime.serviceError == null && System.currentTimeMillis() < until) {
                        Thread.sleep(25)
                    }
                    UsbRuntime.serviceError?.let { return@execute invoke.reject(it) }
                    if (!UsbRuntime.serviceRunning) return@execute invoke.reject("Foreground service did not start in time.")
                }
                invoke.resolve()
            } catch (e: Exception) {
                invoke.reject("Foreground service could not start: ${e.javaClass.simpleName}: ${e.message}")
            }
        }
    }

    @Command
    fun updateService(invoke: Invoke) {
        val args = invoke.parseArgs(ServiceArgs::class.java)
        PollingService.instance?.update(args.label, args.state, args.devices)
        invoke.resolve()
    }

    @Command
    fun stopService(invoke: Invoke) {
        PollingService.instance?.stopSelf()
        invoke.resolve()
    }

    @Command
    fun serviceStatus(invoke: Invoke) {
        val o = notificationState()
        o.put("running", UsbRuntime.serviceRunning)
        UsbRuntime.lastEvent?.let { o.put("lastEvent", it) }
        invoke.resolve(o)
    }

    // ---- export through SAF ----------------------------------------------

    private fun exportDirectory(): File {
        val dir = File(activity.cacheDir, "exports")
        dir.mkdirs()
        // Leftovers from a crashed save are removed after a day.
        val cutoff = System.currentTimeMillis() - 24L * 3600 * 1000
        dir.listFiles()?.forEach { if (it.isFile && it.lastModified() < cutoff) it.delete() }
        return dir
    }

    @Command
    fun exportDir(invoke: Invoke) {
        val o = JSObject()
        o.put("path", exportDirectory().absolutePath)
        invoke.resolve(o)
    }

    @Command
    fun saveDocument(invoke: Invoke) {
        val args = invoke.parseArgs(SaveArgs::class.java)
        val ext = args.cacheFileName.substringAfterLast('.', "").lowercase()
        val mime = MIME[ext] ?: return invoke.reject("File type .$ext cannot be exported.")
        if (!SAFE_NAME.matches(args.cacheFileName) || args.mime != mime) {
            return invoke.reject("Invalid export file.")
        }
        val file = File(exportDirectory(), args.cacheFileName)
        if (!file.isFile) return invoke.reject("The export file is missing.")
        synchronized(lock) {
            if (pendingSave != null) return invoke.reject("A save dialog is already open.")
            pendingSave = PendingSave(file, mime)
        }
        val intent = Intent(Intent.ACTION_CREATE_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType(mime)
            .putExtra(Intent.EXTRA_TITLE, args.displayName.take(120))
        try {
            startActivityForResult(invoke, intent, "saveResult")
        } catch (e: Exception) {
            finishSave()?.delete()
            invoke.reject("Could not open the save dialog: ${e.message}")
        }
    }

    private fun finishSave(): File? = synchronized(lock) {
        val p = pendingSave
        pendingSave = null
        p?.cacheFile
    }

    @ActivityCallback
    fun saveResult(invoke: Invoke, result: ActivityResult) {
        val file = finishSave()
        val uri: Uri? = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null || file == null) {
            file?.delete()
            val o = JSObject()
            o.put("saved", false)
            o.put("message", "not saved")
            return invoke.resolve(o)
        }
        io.execute {
            try {
                val out = activity.contentResolver.openOutputStream(uri, "wt")
                    ?: throw IOException("the document cannot be opened for writing")
                out.use { o -> file.inputStream().use { it.copyTo(o) } }
                val o = JSObject()
                o.put("saved", true)
                invoke.resolve(o)
            } catch (e: Exception) {
                try {
                    DocumentsContract.deleteDocument(activity.contentResolver, uri)
                } catch (_: Exception) {
                }
                invoke.reject("Could not save the file: ${e.message}")
            } finally {
                file.delete()
            }
        }
    }

    companion object {
        const val EXTRA_REQUEST_ID = "buslinkRequestId"
        const val PERMISSION_TIMEOUT_MS = 60_000L
        private val SAFE_NAME = Regex("^[A-Za-z0-9][A-Za-z0-9._-]{0,79}$")
        private val MIME = mapOf(
            "csv" to "text/csv",
            "xlsx" to "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "json" to "application/json",
            "html" to "text/html",
        )
    }
}

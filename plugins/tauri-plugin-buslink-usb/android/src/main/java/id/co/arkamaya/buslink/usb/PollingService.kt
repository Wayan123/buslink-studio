package id.co.arkamaya.buslink.usb

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

/**
 * Foreground service of type connectedDevice. It only keeps the process alive
 * while the Rust core polls; it never polls, arms or writes by itself.
 * START_NOT_STICKY: if the system kills it, nothing restarts on its own.
 */
class PollingService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null
    private var lastNotify = 0L

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        instance = this
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= 26) {
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL, getString(R.string.buslink_channel_name), NotificationManager.IMPORTANCE_LOW),
            )
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_DISCONNECT) {
            UsbRuntime.closeAll("Disconnected from the notification")
            stopSelf()
            return START_NOT_STICKY
        }
        val label = intent?.getStringExtra(EXTRA_LABEL) ?: ""
        val state = intent?.getStringExtra(EXTRA_STATE) ?: "stopped"
        val devices = intent?.getIntExtra(EXTRA_DEVICES, 0) ?: 0
        try {
            ServiceCompat.startForeground(
                this,
                NOTIF_ID,
                build(label, state, devices),
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE,
            )
        } catch (e: Exception) {
            // Missing type permission (SecurityException on API 34+) or a start from the background.
            // No fallback to another service type (dataSync is capped at 6 h on API 35).
            UsbRuntime.serviceError = "Foreground service could not start: ${e.javaClass.simpleName}: ${e.message}"
            UsbRuntime.event(UsbRuntime.serviceError ?: "foreground service failed")
            stopSelf()
            return START_NOT_STICKY
        }
        UsbRuntime.serviceError = null
        UsbRuntime.serviceRunning = true
        lastNotify = System.currentTimeMillis()
        applyWakeLock(state)
        return START_NOT_STICKY
    }

    /** Called by the plugin (same process) so no new foreground-service start is needed. */
    fun update(label: String, state: String, devices: Int) {
        val now = System.currentTimeMillis()
        if (now - lastNotify >= MIN_NOTIFY_MS) {
            lastNotify = now
            val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            nm.notify(NOTIF_ID, build(label, state, devices))
        }
        // Refreshed on every update; expires by itself if the app stops reporting.
        applyWakeLock(state)
    }

    private fun applyWakeLock(state: String) {
        if (state == "polling") {
            val lock = wakeLock ?: (getSystemService(Context.POWER_SERVICE) as PowerManager)
                .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "buslink:polling")
                .also {
                    it.setReferenceCounted(false)
                    wakeLock = it
                }
            lock.acquire(WAKE_LOCK_MS)
        } else {
            wakeLock?.let { if (it.isHeld) it.release() }
        }
    }

    private fun build(label: String, state: String, devices: Int) = run {
        val text = when (state) {
            "polling" -> getString(R.string.buslink_state_polling, label, devices)
            "disconnected" -> getString(R.string.buslink_state_disconnected, label)
            else -> getString(R.string.buslink_state_stopped, label)
        }
        val open = packageManager.getLaunchIntentForPackage(packageName)?.let {
            PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
        val disconnect = PendingIntent.getService(
            this,
            1,
            Intent(this, PollingService::class.java).setAction(ACTION_DISCONNECT),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_buslink_notification)
            .setContentTitle(getString(R.string.buslink_notif_title))
            .setContentText(text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setContentIntent(open)
            .addAction(0, getString(R.string.buslink_action_disconnect), disconnect)
            .build()
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        // The user swiped the app away: close the ports and stop (app exit).
        UsbRuntime.closeAll("App closed")
        stopSelf()
        super.onTaskRemoved(rootIntent)
    }

    override fun onDestroy() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        UsbRuntime.serviceRunning = false
        if (instance === this) instance = null
        super.onDestroy()
    }

    companion object {
        const val CHANNEL = "buslink_polling"
        const val NOTIF_ID = 4201
        const val ACTION_DISCONNECT = "id.co.arkamaya.buslink.usb.DISCONNECT"
        const val EXTRA_LABEL = "label"
        const val EXTRA_STATE = "state"
        const val EXTRA_DEVICES = "devices"
        const val MIN_NOTIFY_MS = 4_000L
        const val WAKE_LOCK_MS = 30_000L

        @Volatile var instance: PollingService? = null
    }
}

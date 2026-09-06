package app.hyperbola.ytdlp

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat

/**
 * Keeps downloads alive while the phone is asleep.
 *
 * Android stops an app's work within seconds of the screen going off, which
 * is fatal for a download measured in minutes. A foreground service with a
 * visible notification is the platform's own answer: the work continues, and
 * the user can see what is holding the phone awake and stop it.
 */
class DownloadService : Service() {
    private var wakeLock: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val text = intent?.getStringExtra(EXTRA_TEXT) ?: "Downloading"
        startForeground(NOTIFICATION_ID, buildNotification(text))
        if (wakeLock?.isHeld != true) {
            val power = getSystemService(Context.POWER_SERVICE) as PowerManager
            wakeLock = power.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "hyperbola:downloads")
                .apply { acquire(MAX_HOLD_MILLIS) }
        }
        // Not sticky: a download that died with the process cannot be resumed
        // by restarting an empty service. The queue on disk brings it back.
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        super.onDestroy()
    }

    private fun buildNotification(text: String): Notification {
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Downloads",
                NotificationManager.IMPORTANCE_LOW,
            )
            channel.description = "Shown while downloads are running"
            manager.createNotificationChannel(channel)
        }
        val open = packageManager.getLaunchIntentForPackage(packageName)?.let {
            PendingIntent.getActivity(
                this,
                0,
                it,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
        }
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Hyperbola")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setContentIntent(open)
            .build()
    }

    companion object {
        const val EXTRA_TEXT = "text"
        private const val CHANNEL_ID = "hyperbola-downloads"
        private const val NOTIFICATION_ID = 1001
        /** A ceiling, not a target: the service releases the lock when it stops. */
        private const val MAX_HOLD_MILLIS = 6L * 60 * 60 * 1000

        fun start(context: Context, text: String) {
            val intent = Intent(context, DownloadService::class.java).putExtra(EXTRA_TEXT, text)
            ContextCompat.startForegroundService(context, intent)
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, DownloadService::class.java))
        }
    }
}

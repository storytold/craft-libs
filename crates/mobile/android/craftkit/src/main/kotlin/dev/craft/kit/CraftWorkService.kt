package dev.craft.kit

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat

/**
 * Keeps the app's process running while it finishes work the user is waiting for (an export
 * encoding on a worker, recovery copies of unsaved work written as the app left the screen), even
 * after the user leaves it: craft-mobile's busy request (wire kind 9).
 *
 * The work itself runs in the app's own threads; Android only has to not freeze or end the
 * process meanwhile, which a cached (background) process gets. A short foreground service is the
 * smallest thing that says so: no permission beyond `FOREGROUND_SERVICE`, a notification while it
 * runs, and at most about three minutes (API 34+; `onTimeout` ends it), which an export never needs.
 * WorkManager would mean moving the work out of the app, and an `expedited` job still couldn't
 * reach the app's in-memory document.
 *
 * Main thread only: [hold] and [release] follow the busy requests in order.
 */
class CraftWorkService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // Always in the foreground first: a service started this way must get there, even if the
        // work ended meanwhile.
        val type = if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE else 0
        ServiceCompat.startForeground(this, NOTIFICATION_ID, notification(), type)
        running = this
        if (!wanted) finish()
        return START_NOT_STICKY
    }

    /** API 35+: the short service's time is up; Android ends the app unless the service stops now. */
    override fun onTimeout(startId: Int, fgsType: Int) {
        Log.w(TAG, "background time for the app's work ran out")
        finish()
    }

    @Deprecated("API 34 only; API 35 calls onTimeout(startId, fgsType)", ReplaceWith("onTimeout(startId, 0)"))
    override fun onTimeout(startId: Int) = onTimeout(startId, 0)

    override fun onDestroy() {
        if (running === this) running = null
        super.onDestroy()
    }

    private fun finish() {
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        stopSelf()
        if (running === this) running = null
    }

    private fun notification(): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26 && manager.getNotificationChannel(CHANNEL) == null) {
            manager.createNotificationChannel(NotificationChannel(CHANNEL, "Finishing work", NotificationManager.IMPORTANCE_LOW))
        }
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.craft_ic_save)
            .setContentTitle(applicationInfo.loadLabel(packageManager))
            .setContentText("Saving your work…")
            .setProgress(0, 0, true)
            .setOngoing(true)
            .setSilent(true)
            .build()
    }

    companion object {
        private const val TAG = "craftkit"
        private const val CHANNEL = "craftkit-work"
        private const val NOTIFICATION_ID = 0xC4AF7

        /** Whether the app is busy, as it last said. */
        private var wanted = false

        /** The service once it's in the foreground. */
        private var running: CraftWorkService? = null

        /** The app started work that must finish: keep the process running until [release]. */
        fun hold(context: Context) {
            wanted = true
            if (running != null) return
            runCatching { ContextCompat.startForegroundService(context, Intent(context, CraftWorkService::class.java)) }
                // Only an app in the foreground may start one; work begun in the background goes without.
                .onFailure { Log.w(TAG, "no background time for the app's work", it) }
        }

        /** The work is done (or abandoned). */
        fun release() {
            wanted = false
            running?.finish()
        }
    }
}

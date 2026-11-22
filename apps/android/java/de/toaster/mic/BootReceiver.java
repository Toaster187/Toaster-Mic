package de.toaster.mic;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;

/**
 * Android 14+ forbids starting a microphone service after boot without user
 * interaction. One tap on this notification restores Always-On standby.
 */
public class BootReceiver extends BroadcastReceiver {
    @Override public void onReceive(Context c, Intent intent) {
        if (Intent.ACTION_BOOT_COMPLETED.equals(intent.getAction()) && Prefs.bool(c, "always_on")) notifyReady(c);
    }

    /** Asks the user for the one tap Android requires before mic access from the background. */
    static void notifyReady(Context c) {
        NotificationManager nm = c.getSystemService(NotificationManager.class);
        nm.createNotificationChannel(new NotificationChannel("ready", "Bereitschaft", NotificationManager.IMPORTANCE_DEFAULT));
        PendingIntent standby = PendingIntent.getForegroundService(c, 3,
            new Intent(c, MicService.class).setAction(MicService.STANDBY), PendingIntent.FLAG_IMMUTABLE);
        nm.notify(8126, new Notification.Builder(c, "ready")
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentTitle("Toaster Mic bereitstellen")
            .setContentText("Tippen, damit der PC das Handy-Mikrofon wieder wecken kann.")
            .setContentIntent(standby).setAutoCancel(true).build());
    }
}

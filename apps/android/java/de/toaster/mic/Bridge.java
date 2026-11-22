package de.toaster.mic;

import android.Manifest;
import android.app.Activity;
import android.app.NotificationManager;
import android.content.ActivityNotFoundException;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.provider.Settings;

/** Small static helpers the Rust UI calls through JNI. */
public final class Bridge {
    private Bridge() { }
    public static void send(Context c, String action, String key, int value) {
        Intent i = new Intent(c, MicService.class).setAction(action).putExtra("key", key).putExtra("value", value);
        if (MicService.START.equals(action) || MicService.STANDBY.equals(action)) c.startForegroundService(i);
        else c.startService(i);
    }
    public static boolean canRecord(Context c) {
        return c.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED;
    }
    public static void requestPermissions(Activity a) {
        if (Build.VERSION.SDK_INT >= 33) a.requestPermissions(new String[]{Manifest.permission.RECORD_AUDIO, Manifest.permission.POST_NOTIFICATIONS}, 1);
        else a.requestPermissions(new String[]{Manifest.permission.RECORD_AUDIO}, 1);
    }
    public static boolean dndAccess(Context c) {
        return c.getSystemService(NotificationManager.class).isNotificationPolicyAccessGranted();
    }
    public static void openDndAccess(Activity a) {
        open(a, new Intent(Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS));
    }
    public static void openBatterySettings(Activity a) {
        open(a, new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS));
    }
    private static void open(Activity a, Intent i) {
        try { a.startActivity(i); } catch (ActivityNotFoundException e) { a.startActivity(new Intent(Settings.ACTION_SETTINGS)); }
    }
}

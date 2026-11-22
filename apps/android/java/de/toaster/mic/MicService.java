package de.toaster.mic;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.net.ConnectivityManager;
import android.net.LinkProperties;
import android.net.Network;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;

/**
 * Foreground service of the :audio process. Android lifecycle glue only:
 * notification, wake/WLAN locks, do-not-disturb and network callbacks.
 * Capture, encryption, network protocol and standby live in Rust (Native).
 */
public class MicService extends Service {
    public static final String START = "de.toaster.mic.START", STANDBY = "de.toaster.mic.STANDBY",
        STOP = "de.toaster.mic.STOP", MUTE = "de.toaster.mic.MUTE", SET = "de.toaster.mic.SET";
    static final int EVENT_CAPTURING = 1, EVENT_IDLE = 2, EVENT_STOPPED = 3;
    private static final int NOTIFICATION = 8125;
    private static volatile MicService instance;
    private final Handler main = new Handler(Looper.getMainLooper());
    private PowerManager.WakeLock wakeLock;
    private WifiManager.WifiLock wifiLock;
    private ConnectivityManager.NetworkCallback network;
    private boolean running, standby, capturing, muted, live;

    private final Runnable renewWakeLock = new Runnable() {
        @Override public void run() {
            if (capturing && wakeLock != null) {
                wakeLock.acquire(10 * 60 * 1000L);
                main.postDelayed(this, 5 * 60 * 1000L);
            }
        }
    };

    static void post(int event, int profile) {
        MicService s = instance;
        if (s != null) s.main.post(() -> s.onEngineEvent(event, profile));
    }

    @Override public void onCreate() {
        super.onCreate();
        instance = this;
        NotificationManager nm = getSystemService(NotificationManager.class);
        nm.createNotificationChannel(new NotificationChannel("microphone", "Mikrofon-Übertragung", NotificationManager.IMPORTANCE_LOW));
        LiveMode.set(this, false); // clean up after a crash
    }

    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent == null) {
            // Restarted by the system after the process was killed: Android does
            // not grant microphone access without a user interaction then.
            if (Prefs.bool(this, "always_on")) BootReceiver.notifyReady(this);
            shutdown();
            return START_NOT_STICKY;
        }
        String action = intent.getAction();
        if (action == null) action = STOP;
        switch (action) {
            case START:
            case STANDBY: {
                boolean wantStandby = STANDBY.equals(action);
                foreground(wantStandby);
                if (running) Native.stop();
                muted = Prefs.bool(this, "muted");
                live = Prefs.bool(this, "live");
                standby = wantStandby;
                running = Native.start(this, wantStandby) == 0;
                if (!running) { shutdown(); return START_NOT_STICKY; }
                watchNetwork();
                getSystemService(NotificationManager.class).cancel(8126);
                return START_NOT_STICKY;
            }
            case MUTE:
                if (!running) { stopIfIdle(); return START_NOT_STICKY; }
                muted = !muted;
                Native.setMuted(muted);
                refreshNotification();
                return START_NOT_STICKY;
            case SET: {
                if (!running) { stopIfIdle(); return START_NOT_STICKY; }
                String key = intent.getStringExtra("key");
                int value = intent.getIntExtra("value", 0);
                if ("muted".equals(key)) { muted = value != 0; Native.setMuted(muted); refreshNotification(); }
                else if ("gain".equals(key)) Native.setGain(value);
                else if ("left".equals(key)) Native.setLeft(value);
                else if ("live".equals(key)) { live = value != 0; Native.setLive(live); if (capturing) LiveMode.set(this, live); }
                return START_NOT_STICKY;
            }
            default:
                shutdown();
                return START_NOT_STICKY;
        }
    }

    private void stopIfIdle() { if (!running) stopSelf(); }

    private void onEngineEvent(int event, int profile) {
        switch (event) {
            case EVENT_CAPTURING:
                capturing = true;
                acquireLocks(profile);
                if (live) LiveMode.set(this, true);
                break;
            case EVENT_IDLE:
                capturing = false;
                releaseLocks();
                LiveMode.set(this, false);
                break;
            default:
                capturing = false;
                running = false;
                shutdown();
                return;
        }
        refreshNotification();
    }

    private void acquireLocks(int profile) {
        if (wakeLock == null) {
            wakeLock = getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "ToasterMic:Capture");
            wakeLock.setReferenceCounted(false);
        }
        wakeLock.acquire(10 * 60 * 1000L);
        main.removeCallbacks(renewWakeLock);
        main.postDelayed(renewWakeLock, 5 * 60 * 1000L);
        if (wifiLock != null && wifiLock.isHeld()) wifiLock.release();
        wifiLock = null;
        // Battery profile lets the radio save power between bursts.
        if (profile < 2) {
            WifiManager wifi = (WifiManager) getApplicationContext().getSystemService(WIFI_SERVICE);
            int mode = profile == 0 && Build.VERSION.SDK_INT >= 29 ? WifiManager.WIFI_MODE_FULL_LOW_LATENCY : WifiManager.WIFI_MODE_FULL_HIGH_PERF;
            wifiLock = wifi.createWifiLock(mode, "ToasterMic:Wifi");
            wifiLock.acquire();
        }
    }

    private void releaseLocks() {
        main.removeCallbacks(renewWakeLock);
        if (wakeLock != null && wakeLock.isHeld()) wakeLock.release();
        if (wifiLock != null && wifiLock.isHeld()) wifiLock.release();
        wifiLock = null;
    }

    private void watchNetwork() {
        if (network != null) return;
        ConnectivityManager cm = getSystemService(ConnectivityManager.class);
        network = new ConnectivityManager.NetworkCallback() {
            @Override public void onAvailable(Network n) { Native.networkChanged(); }
            @Override public void onLinkPropertiesChanged(Network n, LinkProperties p) { Native.networkChanged(); }
        };
        try { cm.registerDefaultNetworkCallback(network); } catch (RuntimeException e) { network = null; }
    }

    private void foreground(boolean idle) {
        Notification n = notification(idle);
        if (Build.VERSION.SDK_INT >= 30) startForeground(NOTIFICATION, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE);
        else startForeground(NOTIFICATION, n);
    }

    private void refreshNotification() {
        if (running) getSystemService(NotificationManager.class).notify(NOTIFICATION, notification(standby && !capturing));
    }

    private Notification notification(boolean idle) {
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, ToasterActivity.class), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        PendingIntent mute = PendingIntent.getService(this, 1, new Intent(this, MicService.class).setAction(MUTE), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        PendingIntent stop = PendingIntent.getService(this, 2, new Intent(this, MicService.class).setAction(STOP), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification.Builder b = new Notification.Builder(this, "microphone")
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true);
        if (idle) {
            b.setContentTitle("Toaster Mic · bereit").setContentText("Always-On: der PC weckt das Mikrofon bei Bedarf.");
        } else {
            b.setContentTitle("Toaster Mic · " + (muted ? "stumm" : "Mikrofon aktiv")).setContentText("WLAN-Übertragung. Tippen zum Öffnen.")
                .addAction(new Notification.Action.Builder(null, muted ? "Ton an" : "Stumm", mute).build());
        }
        return b.addAction(new Notification.Action.Builder(null, "Beenden", stop).build()).build();
    }

    private void shutdown() {
        if (running) Native.stop();
        running = capturing = false;
        releaseLocks();
        LiveMode.set(this, false);
        if (network != null) {
            try { getSystemService(ConnectivityManager.class).unregisterNetworkCallback(network); } catch (RuntimeException ignored) { }
            network = null;
        }
        stopForeground(STOP_FOREGROUND_REMOVE);
        stopSelf();
    }

    @Override public void onDestroy() {
        if (running) Native.stop();
        running = false;
        releaseLocks();
        LiveMode.set(this, false);
        main.removeCallbacksAndMessages(null);
        instance = null;
        super.onDestroy();
    }

    @Override public IBinder onBind(Intent intent) { return null; }
}

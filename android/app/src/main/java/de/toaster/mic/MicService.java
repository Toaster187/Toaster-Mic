package de.toaster.mic;

import android.app.*;
import android.Manifest;
import android.content.*;
import android.content.pm.PackageManager;
import android.content.pm.ServiceInfo;
import android.media.*;
import android.net.wifi.WifiManager;
import android.os.*;
import android.util.Pair;
import java.net.*;
import java.nio.*;
import java.util.*;

public class MicService extends Service {
    public static final String START = "de.toaster.mic.START", STOP = "de.toaster.mic.STOP", MUTE = "de.toaster.mic.MUTE";
    private static final int NOTIFICATION = 8125;
    private volatile boolean stopping;
    private volatile DatagramSocket socket;
    private volatile AudioRecord recorder;
    private boolean pixelPair;
    private PowerManager.WakeLock wakeLock;
    private WifiManager.WifiLock wifiLock;
    private Thread worker;
    private final Handler main = new Handler(Looper.getMainLooper());
    private volatile Peer peer;
    private volatile long lastHello;
    private final Runnable renewWakeLock = new Runnable() {
        @Override public void run() {
            if (!stopping && State.running && wakeLock != null) {
                wakeLock.acquire(10 * 60 * 1000L);
                main.postDelayed(this, 5 * 60 * 1000L);
            }
        }
    };
    private record Peer(InetAddress address, int port, long session, byte[] request, byte[] reply, Wire sender, Wire control) {
        boolean matches(DatagramPacket packet) { return address.equals(packet.getAddress()) && port == packet.getPort(); }
    }

    @Override public void onCreate() {
        super.onCreate();
        getSystemService(NotificationManager.class).createNotificationChannel(new NotificationChannel("microphone", "Mikrofon-Übertragung", NotificationManager.IMPORTANCE_LOW));
    }
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        String action = intent == null ? STOP : intent.getAction();
        if (STOP.equals(action)) { stopSelf(); return START_NOT_STICKY; }
        if (MUTE.equals(action)) {
            if (!State.running) { stopSelf(); return START_NOT_STICKY; }
            State.muted = !State.muted;
            getSharedPreferences("mic", MODE_PRIVATE).edit().putBoolean("muted", State.muted).apply();
            notifyState(); return START_NOT_STICKY;
        }
        if (State.running) return START_NOT_STICKY;
        SharedPreferences p = getSharedPreferences("mic", MODE_PRIVATE);
        State.muted = p.getBoolean("muted", false); State.gain = p.getInt("gain", 100) / 100f;
        State.left = Math.max(0, Math.min(1, p.getInt("left", 0))); State.right = 1 - State.left;
        State.stereoVerified = false;
        try {
            if (Build.VERSION.SDK_INT >= 30) startForeground(NOTIFICATION, notification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE);
            else startForeground(NOTIFICATION, notification());
            wakeLock = getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "ToasterMic:Capture");
            wakeLock.setReferenceCounted(false);
            wakeLock.acquire(10 * 60 * 1000L);
            WifiManager wifi = (WifiManager)getApplicationContext().getSystemService(WIFI_SERVICE);
            wifiLock = wifi.createWifiLock(Build.VERSION.SDK_INT >= 29 ? WifiManager.WIFI_MODE_FULL_LOW_LATENCY : WifiManager.WIFI_MODE_FULL_HIGH_PERF, "ToasterMic:Wifi");
            wifiLock.acquire();
            stopping = false; State.running = true; State.status = "Mikrofon wird vorbereitet …";
            main.postDelayed(renewWakeLock, 5 * 60 * 1000L);
            worker = new Thread(() -> capture(p), "ToasterMic-capture"); worker.start();
        } catch (Exception e) { State.status = "Start fehlgeschlagen: " + e.getMessage(); stopSelf(); }
        return START_NOT_STICKY;
    }
    private void capture(SharedPreferences settings) {
        android.os.Process.setThreadPriority(android.os.Process.THREAD_PRIORITY_URGENT_AUDIO);
        Thread controls = null;
        try {
            if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED)
                throw new SecurityException("Mikrofonberechtigung fehlt oder wurde entzogen.");
            int channels = settings.getBoolean("stereo", false) ? 2 : 1;
            int mask = channels == 2 ? AudioFormat.CHANNEL_IN_STEREO : AudioFormat.CHANNEL_IN_MONO;
            int minBuffer = AudioRecord.getMinBufferSize(Wire.RATE, mask, AudioFormat.ENCODING_PCM_16BIT);
            if (minBuffer <= 0) throw new IllegalStateException("48-kHz-Aufnahme wird nicht unterstützt. Mono oder anderes Eingabegerät wählen.");
            AudioManager audio = getSystemService(AudioManager.class);
            int deviceId = channels == 2 ? settings.getInt("device", 0) : 0;
            AudioDeviceInfo selected = null;
            for (AudioDeviceInfo device : audio.getDevices(AudioManager.GET_DEVICES_INPUTS))
                if (device.getId() == deviceId && MicrophoneRouting.isMicrophone(device)) selected = device;
            pixelPair = channels == 2 && PixelMicrophones.supported() && (selected == null || selected.getType() == AudioDeviceInfo.TYPE_BUILTIN_MIC);
            if (pixelPair && !PixelMicrophones.calibrated(settings)) throw new IllegalStateException("Bitte zuerst oben, unten und hinten unter Einstellungen zuordnen.");
            int captureChannels = pixelPair ? 3 : channels;
            int bottom = pixelPair ? settings.getInt("pixelBottom", -1) : 0;
            int top = pixelPair ? settings.getInt("pixelTop", -1) : 1;
            if (pixelPair) selected = PixelMicrophones.input(audio);
            boolean raw = "true".equals(audio.getProperty(AudioManager.PROPERTY_SUPPORT_AUDIO_SOURCE_UNPROCESSED));
            // Pixel's third raw input needs a channel index mask and physical calibration.
            recorder = new AudioRecord.Builder().setAudioSource(channels == 2 || raw ? MediaRecorder.AudioSource.UNPROCESSED : MediaRecorder.AudioSource.VOICE_RECOGNITION)
                .setAudioFormat(pixelPair ? PixelMicrophones.format() : new AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT).setSampleRate(Wire.RATE).setChannelMask(mask).build())
                .setBufferSizeInBytes(Math.max(minBuffer, Wire.FRAMES * captureChannels * 2 * 4)).build();
            if (recorder.getState() != AudioRecord.STATE_INITIALIZED) throw new IllegalStateException("Aufnahme nicht verfügbar. Stereo ausschalten oder Eingabegerät wechseln.");
            if (selected == null) settings.edit().putInt("device", 0).apply();
            else if (!recorder.setPreferredDevice(selected)) throw new IllegalStateException("Gewähltes Mikrofon nicht verfügbar.");
            socket = new DatagramSocket(null); socket.setReuseAddress(false);
            socket.bind(new InetSocketAddress(settings.getInt("port", 8125))); socket.setSoTimeout(500);
            recorder.startRecording();
            if (recorder.getRecordingState() != AudioRecord.RECORDSTATE_RECORDING) throw new IllegalStateException("Android hat die Mikrofonaufnahme nicht gestartet.");
            if (channels == 2) {
                short[] warmup = new short[Wire.FRAMES * captureChannels];
                for (int i = 0; i < 40 && !stopping; i++)
                    if (recorder.read(warmup, 0, warmup.length, AudioRecord.READ_BLOCKING) != warmup.length) throw new IllegalStateException("Stereo-Aufnahme unterbrochen.");
                if (stopping) return;
            }
            updateMicrophones(channels);
            controls = new Thread(this::receiveControl, "ToasterMic-control"); controls.start();
            short[] samples = new short[Wire.FRAMES * captureChannels];
            byte[] pcm = new byte[Wire.FRAMES * channels * 2];
            ByteBuffer payload = ByteBuffer.wrap(pcm).order(ByteOrder.LITTLE_ENDIAN);
            int sequence = 0, ticks = 0;
            State.status = "Bereit · am PC mit Handy-IP verbinden";
            while (!stopping) {
                int read = 0;
                while (read < samples.length && !stopping) {
                    int count = recorder.read(samples, read, samples.length - read, AudioRecord.READ_BLOCKING);
                    if (count <= 0) throw new IllegalStateException("Mikrofon unterbrochen (" + count + "). Bitte erneut starten.");
                    read += count;
                }
                if (stopping) break;
                boolean muted = State.muted; float gain = State.gain;
                int left = channels == 1 ? 0 : Math.max(0, Math.min(1, State.left));
                int right = 1 - left;
                if (pixelPair) { right = left == 0 ? top : bottom; left = left == 0 ? bottom : top; }
                int peak = 0, leftPeak = 0, rightPeak = 0; payload.clear();
                for (int frame = 0; frame < Wire.FRAMES; frame++) {
                    for (int ch = 0; ch < channels; ch++) {
                        int value = muted ? 0 : Math.round(samples[frame * captureChannels + (ch == 0 ? left : right)] * gain);
                        value = Math.max(-32768, Math.min(32767, value)); peak = Math.max(peak, Math.abs(value)); payload.putShort((short)value);
                        if (ch == 0) leftPeak = Math.max(leftPeak, Math.abs(value)); else rightPeak = Math.max(rightPeak, Math.abs(value));
                    }
                }
                State.peak = peak;
                State.leftPeak = Math.max(leftPeak, State.leftPeak * 97 / 100);
                State.rightPeak = Math.max(rightPeak, State.rightPeak * 97 / 100);
                Peer target = peer;
                if (target != null && SystemClock.elapsedRealtime() - lastHello < 3500) {
                    // Never reuse a GCM nonce, including when an old HELLO is replayed.
                    if (sequence == -1) throw new IllegalStateException("Bitte Mikrofon neu starten (Sitzungsgrenze erreicht).");
                    byte[] packet = target.sender().encode(Wire.AUDIO, channels, muted, target.session(), sequence++, pcm);
                    try { socket.send(new DatagramPacket(packet, packet.length, target.address(), target.port())); }
                    catch (java.io.IOException e) { State.status = "WLAN unterbrochen · warte auf Verbindung …"; }
                    if (ticks % 100 == 0) State.status = (muted ? "Stumm" : channels == 2 ? "Stereo · 2 Mikrofone" : "Sendet Mono") + " → " + target.address().getHostAddress();
                } else { State.status = "Bereit · warte auf PC-Verbindung"; }
                if (++ticks % 400 == 0) updateMicrophones(channels);
            }
        } catch (Exception e) {
            if (!stopping) State.status = "Fehler: " + (e.getMessage() == null ? e.getClass().getSimpleName() : e.getMessage());
        } finally {
            boolean alreadyStopping = stopping;
            stopping = true;
            if (socket != null) socket.close();
            if (controls != null) try { controls.join(1000); } catch (InterruptedException e) { Thread.currentThread().interrupt(); }
            if (recorder != null) { try { recorder.stop(); } catch (IllegalStateException ignored) { } recorder.release(); recorder = null; }
            if (!alreadyStopping) main.post(this::stopSelf);
        }
    }
    private void receiveControl() {
        byte[] data = new byte[128];
        DatagramPacket packet = new DatagramPacket(data, data.length);
        Peer pending = null; long pendingUntil = 0, lastExchange = -1000;
        while (!stopping) {
            try {
                packet.setLength(data.length); socket.receive(packet);
                long now = SystemClock.elapsedRealtime();
                Peer current = peer;
                if (KeyExchange.isOffer(data, packet.getLength(), 1)) {
                    byte[] request = Arrays.copyOf(data, packet.getLength());
                    Peer response;
                    if (current != null && current.matches(packet) && Arrays.equals(current.request(), request)) response = current;
                    else {
                        if (current != null && now - lastHello < 3500) continue;
                        if (pending != null && now < pendingUntil && pending.matches(packet) && Arrays.equals(pending.request(), request)) response = pending;
                        else {
                            if (now - lastExchange < 250) continue;
                            lastExchange = now;
                            KeyExchange exchange = new KeyExchange(); byte[] key = exchange.derive(request);
                            long session = KeyExchange.session(request);
                            response = new Peer(packet.getAddress(), packet.getPort(), session, request, exchange.offer(2, session), new Wire(key), new Wire(key));
                            pending = response; pendingUntil = now + 3500;
                        }
                    }
                    socket.send(new DatagramPacket(response.reply(), response.reply().length, response.address(), response.port()));
                    continue; // No audio or lease renewal until key possession is confirmed.
                }
                Peer candidate = current != null && current.matches(packet) ? current : null;
                Wire.Control command = candidate == null ? null : candidate.control().control(data, packet.getLength());
                if (command == null && pending != null && now < pendingUntil && pending.matches(packet) && (current == null || now - lastHello >= 3500)) {
                    candidate = pending; command = candidate.control().control(data, packet.getLength());
                }
                if (command == null || candidate == null || command.session() != candidate.session()) continue;
                if (command.type() == Wire.HELLO) { lastHello = now; peer = candidate; if (candidate == pending) pending = null; }
                else if (candidate == current) { peer = null; }
            } catch (SocketTimeoutException ignored) {
            } catch (Exception e) { if (!stopping) State.status = "Netzwerk: " + e.getMessage(); }
        }
    }
    private void updateMicrophones(int channels) {
        try {
            if (pixelPair) {
                AudioDeviceInfo route = recorder.getRoutedDevice();
                if (recorder.getChannelCount() != 3 || route == null || route.getType() != AudioDeviceInfo.TYPE_BUILTIN_MIC || !"bottom".equals(route.getAddress()))
                    throw new IllegalStateException("Kalibrierte Pixel-Mikrofonroute hat gewechselt.");
                State.channelNames = new String[]{"Mikrofon unten", "Mikrofon oben"};
                State.microphones = "Physisch zugeordnet: oben + unten. Rückseite wird nicht übertragen.";
                State.stereoVerified = true;
                return;
            }
            List<MicrophoneInfo> active = recorder.getActiveMicrophones();
            String[] directPair = channels == 2 ? MicrophoneRouting.distinctDirectPair(active) : null;
            if (channels == 2 && directPair == null) throw new IllegalStateException("Android gibt keine zwei getrennten Mikrofone frei. Anderen Eingang oder Mono wählen.");
            State.stereoVerified = channels == 2;
            String[] names = {"Kanal 1", "Kanal 2"};
            StringBuilder details = new StringBuilder();
            AudioDeviceInfo routed = recorder.getRoutedDevice();
            if (routed != null) details.append("Eingang: ").append(routed.getProductName()).append("\n");
            for (MicrophoneInfo mic : active) {
                for (Pair<Integer, Integer> mapping : mic.getChannelMapping()) {
                    if (mapping.first < 0 || mapping.first >= channels) continue;
                    String label = mic.getDescription() + " #" + mic.getId() + " · " + location(mic.getLocation());
                    names[mapping.first] += " / " + label;
                    details.append("Kanal ").append(mapping.first + 1).append(": ").append(label)
                        .append(mapping.second == MicrophoneInfo.CHANNEL_MAPPING_DIRECT ? " (direkt)" : " (verarbeitet)").append("\n");
                }
            }
            if (details.length() == 0) details.append("Android meldet keine physischen Mikrofonpositionen. Die Kanalzuordnung bleibt verfügbar.");
            State.channelNames = directPair != null ? directPair : names; State.microphones = details.toString().trim();
        } catch (Exception e) {
            State.stereoVerified = false;
            State.microphones = "Mikrofonpositionen vom Gerät nicht verfügbar.";
            if (channels == 2) throw new IllegalStateException("Zwei getrennte Mikrofone nicht bestätigt: " + e.getMessage(), e);
        }
    }
    private String location(int value) {
        return switch (value) { case MicrophoneInfo.LOCATION_MAINBODY -> "Gehäuse"; case MicrophoneInfo.LOCATION_MAINBODY_MOVABLE -> "bewegliches Gehäuse"; case MicrophoneInfo.LOCATION_PERIPHERAL -> "extern"; default -> "Position unbekannt"; };
    }
    private Notification notification() {
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        PendingIntent mute = PendingIntent.getService(this, 1, new Intent(this, MicService.class).setAction(MUTE), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        PendingIntent stop = PendingIntent.getService(this, 2, new Intent(this, MicService.class).setAction(STOP), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        return new Notification.Builder(this, "microphone").setContentTitle("Toaster Mic · " + (State.muted ? "stumm" : "Mikrofon aktiv"))
            .setContentText("WLAN-Übertragung ist bereit. Tippen zum Öffnen.").setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true)
            .addAction(new Notification.Action.Builder(null, State.muted ? "Ton an" : "Stumm", mute).build())
            .addAction(new Notification.Action.Builder(null, "Stoppen", stop).build()).build();
    }
    private void notifyState() { getSystemService(NotificationManager.class).notify(NOTIFICATION, notification()); }
    @Override public void onDestroy() {
        stopping = true;
        if (socket != null) socket.close();
        AudioRecord active = recorder;
        if (active != null) try { active.stop(); } catch (IllegalStateException ignored) { }
        if (worker != null) try { worker.join(1500); } catch (InterruptedException e) { Thread.currentThread().interrupt(); }
        main.removeCallbacksAndMessages(null);
        if (wakeLock != null && wakeLock.isHeld()) wakeLock.release();
        if (wifiLock != null && wifiLock.isHeld()) wifiLock.release();
        State.running = State.stereoVerified = false; State.peak = State.leftPeak = State.rightPeak = 0;
        if (!State.status.startsWith("Fehler:") && !State.status.startsWith("Start fehlgeschlagen:")) State.status = "Gestoppt";
        stopForeground(STOP_FOREGROUND_REMOVE); super.onDestroy();
    }
    @Override public IBinder onBind(Intent intent) { return null; }
}

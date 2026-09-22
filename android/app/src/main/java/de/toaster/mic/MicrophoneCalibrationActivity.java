package de.toaster.mic;

import android.Manifest;
import android.app.Activity;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.media.*;
import android.os.*;
import android.view.*;
import android.widget.*;
import org.json.*;
import java.io.*;
import java.util.Arrays;

/** User-guided physical identification; capture stays on the phone. */
public class MicrophoneCalibrationActivity extends Activity {
    private TextView prompt, feedback;
    private Button measure;
    private int step;
    private final int[] mapping = {-1, -1, -1}; // top, bottom, rear
    private double[] background;
    private final JSONArray measurements = new JSONArray();
    private volatile boolean cancelled;
    private boolean busy, completed;
    private final String[] instructions = {
        "1 / 4 · Ruhe\n\nHandy ruhig halten und kurz nicht sprechen.",
        "2 / 4 · Mikrofon oben\n\nNach dem Start drei Sekunden leicht direkt über die Mikrofonöffnung an der Oberkante reiben. Nicht über den Lautsprecher.",
        "3 / 4 · Mikrofon unten\n\nNach dem Start drei Sekunden leicht direkt über die Mikrofonöffnung unten reiben.",
        "4 / 4 · Mikrofon hinten\n\nNach dem Start drei Sekunden leicht über die Mikrofonöffnung an der Kamera reiben. Damit wird genau dieser Eingang ausgeschlossen."
    };
    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        LinearLayout root = new LinearLayout(this); root.setOrientation(LinearLayout.VERTICAL);
        int pad = Math.round(24 * getResources().getDisplayMetrics().density); root.setPadding(pad,pad*2,pad,pad);
        root.setBackgroundColor(Color.rgb(17,23,20));
        ScrollView scroll = new ScrollView(this); scroll.addView(root); setContentView(scroll);
        TextView title = text("Mikrofone zuordnen", 26); root.addView(title);
        prompt = text(instructions[0], 19); root.addView(prompt);
        feedback = text("Es werden nur Kanalpegel gespeichert.", 15); root.addView(feedback);
        measure = new Button(this); measure.setText("Messung starten"); root.addView(measure);
        measure.setOnClickListener(v -> {
            if (completed) { finish(); return; }
            if (busy) return;
            if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
                requestPermissions(new String[]{Manifest.permission.RECORD_AUDIO}, 1); return;
            }
            if (State.running) { feedback.setText("Bitte zuerst die Übertragung stoppen."); return; }
            if (!PixelMicrophones.supported()) { feedback.setText("Diese Zuordnung ist für das Pixel 9 vorgesehen."); return; }
            busy = true; cancelled = false; measure.setEnabled(false); feedback.setText("Jetzt messen …");
            new Thread(this::capture, "ToasterMic-calibration").start();
        });
    }
    private TextView text(String value, int size) {
        TextView view = new TextView(this); view.setText(value); view.setTextSize(size); view.setTextColor(Color.WHITE); view.setPadding(0,16,0,16); return view;
    }
    private void capture() {
        AudioRecord record = null;
        try {
            AudioDeviceInfo device = PixelMicrophones.input(getSystemService(AudioManager.class));
            record = new AudioRecord.Builder().setAudioSource(MediaRecorder.AudioSource.UNPROCESSED)
                .setAudioFormat(PixelMicrophones.format()).setBufferSizeInBytes(48000 * 3 * 2 / 10).build();
            if (!record.setPreferredDevice(device)) throw new IOException("Rohmikrofone nicht verfügbar.");
            record.startRecording();
            if (record.getChannelCount() != 3 || record.getRecordingState() != AudioRecord.RECORDSTATE_RECORDING)
                throw new IOException("Drei Rohkanäle sind nicht verfügbar.");
            short[] samples = new short[Wire.FRAMES * 3]; double[] energy = new double[3]; int frames = 0;
            for (int packet = 0; packet < 640 && !cancelled; packet++) {
                int read = 0;
                while (read < samples.length && !cancelled) {
                    int n = record.read(samples, read, samples.length - read, AudioRecord.READ_BLOCKING);
                    if (n <= 0) throw new IOException("Aufnahme unterbrochen."); read += n;
                }
                if (packet < 40 || cancelled) continue;
                for (int frame = 0; frame < Wire.FRAMES; frame++) for (int channel = 0; channel < 3; channel++) {
                    double sample = samples[frame * 3 + channel]; energy[channel] += sample * sample;
                }
                frames += Wire.FRAMES;
            }
            if (cancelled) return;
            AudioDeviceInfo routed = record.getRoutedDevice();
            if (routed == null || routed.getId() != device.getId()) throw new IOException("Mikrofonroute hat gewechselt. Messung wiederholen.");
            double[] rms = new double[3]; for (int i = 0; i < 3; i++) rms[i] = Math.sqrt(energy[i] / frames);
            record.stop(); record.release(); record = null;
            runOnUiThread(() -> accept(rms));
        } catch (Exception e) {
            runOnUiThread(() -> { if (!cancelled) { busy = false; measure.setEnabled(true); feedback.setText(e.getMessage()); } });
        } finally { if (record != null) { try { record.stop(); } catch (Exception ignored) { } record.release(); } }
    }
    private void accept(double[] rms) {
        if (cancelled) return;
        busy = false; measure.setEnabled(true);
        try {
            measurements.put(new JSONObject().put("step", step).put("rms", new JSONArray(rms)));
            try (FileWriter writer = new FileWriter(new File(getFilesDir(), "microphone-calibration.json"))) { writer.write(measurements.toString(2)); }
        } catch (Exception ignored) { }
        if (step == 0) background = rms;
        else {
            int channel = ChannelCalibration.identify(background, rms);
            if (channel < 0) { feedback.setText("Nicht eindeutig. Bitte nur direkt an der genannten Mikrofonöffnung leicht reiben und wiederholen."); return; }
            for (int i = 0; i < step - 1; i++) if (mapping[i] == channel) {
                step = 0; Arrays.fill(mapping, -1); prompt.setText(instructions[0]);
                feedback.setText("Zwei Stellen reagierten auf denselben Kanal. Bitte neu messen und genau die Mikrofonöffnungen verwenden."); return;
            }
            mapping[step - 1] = channel;
        }
        step++;
        if (step < 4) { prompt.setText(instructions[step]); feedback.setText("Messung übernommen."); return; }
        saveMapping();
    }
    @android.annotation.SuppressLint("ApplySharedPref") // Persist the verified map before returning to capture.
    private void saveMapping() {
        if (!ChannelCalibration.distinct(mapping[0], mapping[1], mapping[2])) { feedback.setText("Zuordnung nicht eindeutig."); return; }
        SharedPreferences prefs = getSharedPreferences("mic", MODE_PRIVATE);
        if (!prefs.edit().putInt("pixelTop", mapping[0]).putInt("pixelBottom", mapping[1]).putInt("pixelRear", mapping[2])
            .putString("pixelMapBuild", Build.FINGERPRINT).putInt("device", 0).commit()) {
            step = 3; prompt.setText(instructions[3]); feedback.setText("Speichern fehlgeschlagen. Bitte erneut versuchen."); return;
        }
        State.channelNames = new String[]{"Mikrofon unten", "Mikrofon oben"};
        State.microphones = "Oben und unten zugeordnet. Rückseite wird nicht übertragen.";
        prompt.setText("Fertig · oben und unten"); feedback.setText(State.microphones); measure.setText("Fertig"); completed = true;
    }
    @Override protected void onStop() {
        cancelled = true;
        super.onStop();
        // An interrupted measurement is never accepted or persisted.
        if (!isFinishing()) finish();
    }
}

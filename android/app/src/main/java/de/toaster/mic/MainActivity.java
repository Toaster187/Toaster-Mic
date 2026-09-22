package de.toaster.mic;

import android.Manifest;
import android.app.*;
import android.content.*;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.media.*;
import android.os.*;
import android.provider.Settings;
import android.text.InputType;
import android.view.*;
import android.widget.*;
import java.net.*;
import java.util.*;

public class MainActivity extends Activity {
    private final Handler handler = new Handler(Looper.getMainLooper());
    private SharedPreferences prefs;
    private LinearLayout content;
    private LinearLayout stereoOptions;
    private ScrollView settingsView;
    private TextView status, ip, gainLabel, details;
    private EditText port;
    private Switch stereo;
    private Spinner input, left, right;
    private Button start, mute, refresh;
    private ProgressBar meter, leftMeter, rightMeter;
    private final List<Integer> deviceIds = new ArrayList<>();
    private String[] displayedChannels = new String[0];
    private boolean updating;
    private int refreshTick;
    private final Runnable tick = new Runnable() {
        @Override public void run() {
            status.setText(State.status); details.setText(State.microphones); meter.setProgress(State.peak);
            leftMeter.setProgress(State.leftPeak); rightMeter.setProgress(State.rightPeak);
            start.setText(State.running ? "Stoppen" : "Mikrofon starten");
            mute.setText(State.muted ? "Ton an" : "Stumm");
            port.setEnabled(!State.running); stereo.setEnabled(!State.running); input.setEnabled(!State.running); refresh.setEnabled(!State.running);
            left.setEnabled(stereo.isChecked()); right.setEnabled(stereo.isChecked());
            if (!Arrays.equals(displayedChannels, State.channelNames)) updateChannelNames();
            if (refreshTick++ % 10 == 0) ip.setText(addresses());
            handler.postDelayed(this, 300);
        }
    };
    @Override public void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        prefs = getSharedPreferences("mic", MODE_PRIVATE);
        if (!State.running && PixelMicrophones.calibrated(prefs)) State.channelNames = new String[]{"Mikrofon unten", "Mikrofon oben"};
        if (getIntent().getBooleanExtra("calibrate", false) && (getApplicationInfo().flags & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0)
            handler.post(this::calibrate);
        prefs.edit().remove("code").apply();
        int savedLeft = Math.max(0, Math.min(1, prefs.getInt("left", 0)));
        prefs.edit().putInt("left", savedLeft).putInt("right", 1 - savedLeft).apply();
        State.left = savedLeft; State.right = 1 - savedLeft;
        if (!State.running) { State.muted = prefs.getBoolean("muted", false); State.gain = prefs.getInt("gain", 100) / 100f; }
        getWindow().setStatusBarColor(Color.rgb(17,23,20)); getWindow().setNavigationBarColor(Color.rgb(17,23,20));
        ScrollView scroll = new ScrollView(this); scroll.setFillViewport(true); scroll.setVerticalScrollBarEnabled(false); scroll.setBackgroundColor(Color.rgb(17,23,20));
        content = new LinearLayout(this); content.setOrientation(LinearLayout.VERTICAL); content.setPadding(dp(24),dp(28),dp(24),dp(28)); scroll.addView(content); setContentView(scroll);
        label("Toaster Mic", 30, Color.WHITE);
        button("Einstellungen", v -> showSettings());
        status = label(State.status, 17, Color.rgb(189,244,119));
        label("Handy-IP", 13, Color.LTGRAY);
        ip = label(addresses(), 22, Color.WHITE); ip.setTextIsSelectable(true);
        buildSettings();
        meter = new ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal); meter.setMax(32768); add(meter);
        mute = button(State.muted ? "Ton an" : "Stumm", v -> {
            if (State.running) startService(new Intent(this, MicService.class).setAction(MicService.MUTE));
            else { State.muted = !State.muted; prefs.edit().putBoolean("muted", State.muted).apply(); }
        });
        start = button("Mikrofon starten", v -> {
            if (State.running) { stopService(new Intent(this, MicService.class)); return; }
            if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
                requestPermissions(Build.VERSION.SDK_INT >= 33 ? new String[]{Manifest.permission.RECORD_AUDIO, Manifest.permission.POST_NOTIFICATIONS} : new String[]{Manifest.permission.RECORD_AUDIO}, 1);
            } else {
                if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED && !prefs.getBoolean("notificationAsked", false)) {
                    prefs.edit().putBoolean("notificationAsked", true).apply(); requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 2);
                } else startCapture();
            }
        });
        start.setBackgroundTintList(android.content.res.ColorStateList.valueOf(Color.rgb(189,244,119))); start.setTextColor(Color.rgb(17,23,20));
    }
    private void buildSettings() {
        LinearLayout mainContent = content;
        settingsView = new ScrollView(this);
        content = new LinearLayout(this); content.setOrientation(LinearLayout.VERTICAL); content.setPadding(dp(20),dp(8),dp(20),dp(16));
        settingsView.addView(content);
        label("UDP-Port", 14, Color.LTGRAY);
        port = new EditText(this); port.setInputType(InputType.TYPE_CLASS_NUMBER); port.setSingleLine(true); port.setText(String.valueOf(prefs.getInt("port", 8125))); add(port);
        stereo = new Switch(this); stereo.setText(R.string.stereo); stereo.setTextSize(17); stereo.setChecked(prefs.getBoolean("stereo", false)); add(stereo);
        LinearLayout settingsContent = content;
        stereoOptions = new LinearLayout(this); stereoOptions.setOrientation(LinearLayout.VERTICAL); add(stereoOptions); content = stereoOptions;
        label("Mikrofon / Eingabegerät", 14, Color.LTGRAY);
        input = new Spinner(this); add(input);
        refresh = button("Aktualisieren", v -> refreshDevices());
        refreshDevices();
        if (PixelMicrophones.supported()) button("Mikrofone zuordnen", v -> calibrate());
        label("Links", 14, Color.LTGRAY); left = new Spinner(this); add(left);
        leftMeter = new ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal); leftMeter.setMax(32768); add(leftMeter);
        label("Rechts", 14, Color.LTGRAY); right = new Spinner(this); add(right);
        rightMeter = new ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal); rightMeter.setMax(32768); add(rightMeter);
        updateChannelNames();
        left.setOnItemSelectedListener(channelListener(true)); right.setOnItemSelectedListener(channelListener(false));
        stereoOptions.setVisibility(stereo.isChecked() ? View.VISIBLE : View.GONE);
        stereo.setOnCheckedChangeListener((v, checked) -> {
            stereoOptions.setVisibility(checked ? View.VISIBLE : View.GONE);
            prefs.edit().putBoolean("stereo", checked).apply();
        });
        label("Zwei getrennte Mikrofone · beim Start geprüft", 12, Color.LTGRAY);
        details = label(State.microphones, 12, Color.LTGRAY); details.setVisibility(View.GONE);
        button("Mikrofondetails", v -> details.setVisibility(details.getVisibility() == View.GONE ? View.VISIBLE : View.GONE));
        content = settingsContent;
        gainLabel = label(getString(R.string.gain, prefs.getInt("gain", 100)), 16, Color.WHITE);
        SeekBar gain = new SeekBar(this); gain.setMax(200); gain.setProgress(prefs.getInt("gain", 100)); add(gain);
        gain.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            public void onProgressChanged(SeekBar bar, int value, boolean user) {
                State.gain = value / 100f; gainLabel.setText(getString(R.string.gain, value)); prefs.edit().putInt("gain", value).apply();
            }
            public void onStartTrackingTouch(SeekBar bar) { }
            public void onStopTrackingTouch(SeekBar bar) { }
        });
        button("Akkueinstellungen", v -> {
            try { startActivity(new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS)); }
            catch (ActivityNotFoundException e) { startActivity(new Intent(Settings.ACTION_SETTINGS)); }
        });
        content = mainContent;
    }
    private void showSettings() {
        if (settingsView.getParent() instanceof ViewGroup parent) parent.removeView(settingsView);
        port.setEnabled(!State.running); stereo.setEnabled(!State.running); input.setEnabled(!State.running); refresh.setEnabled(!State.running);
        AlertDialog dialog = new AlertDialog.Builder(this).setTitle("Einstellungen").setView(settingsView).setPositiveButton("Fertig", null).create();
        dialog.setOnShowListener(d -> dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v -> {
            try {
                int value = Integer.parseInt(port.getText().toString().trim());
                if (value < 1024 || value > 65535) throw new NumberFormatException();
                prefs.edit().putInt("port", value).putInt("device", deviceIds.get(Math.max(0, input.getSelectedItemPosition()))).apply();
                dialog.dismiss();
            } catch (NumberFormatException e) { port.setError("Port: 1024–65535"); }
        }));
        dialog.show();
    }
    private void calibrate() {
        if (State.running) { Toast.makeText(this, "Bitte zuerst die Übertragung stoppen.", Toast.LENGTH_SHORT).show(); return; }
        startActivity(new Intent(this, MicrophoneCalibrationActivity.class));
    }
    private void startCapture() {
        try {
            int value = Integer.parseInt(port.getText().toString().trim());
            if (value < 1024 || value > 65535) throw new IllegalArgumentException("Port muss zwischen 1024 und 65535 liegen.");
            int selected = input.getSelectedItemPosition();
            int id = selected >= 0 ? deviceIds.get(selected) : 0;
            boolean internal = true;
            for (AudioDeviceInfo d : getSystemService(AudioManager.class).getDevices(AudioManager.GET_DEVICES_INPUTS))
                if (d.getId() == id && MicrophoneRouting.isMicrophone(d)) internal = d.getType() == AudioDeviceInfo.TYPE_BUILTIN_MIC;
            if (stereo.isChecked() && internal && PixelMicrophones.supported() && !PixelMicrophones.calibrated(prefs)) { calibrate(); return; }
            prefs.edit().putInt("port", value).putBoolean("stereo", stereo.isChecked())
                .putInt("device", selected >= 0 ? deviceIds.get(selected) : 0).apply();
            startForegroundService(new Intent(this, MicService.class).setAction(MicService.START));
        } catch (NumberFormatException e) { State.status = "Bitte eine gültige Portnummer eingeben."; }
        catch (Exception e) { State.status = e.getMessage(); }
    }
    @Override public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(requestCode, permissions, results);
        if (requestCode == 1 || requestCode == 2) {
            prefs.edit().putBoolean("notificationAsked", true).apply();
            if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) startCapture();
            else State.status = "Mikrofonberechtigung fehlt. Bitte in den Android-App-Einstellungen erlauben.";
        }
    }
    private void refreshDevices() {
        List<String> names = new ArrayList<>(); names.add("Automatisch (Android)"); deviceIds.clear(); deviceIds.add(0);
        for (AudioDeviceInfo d : getSystemService(AudioManager.class).getDevices(AudioManager.GET_DEVICES_INPUTS)) {
            if (!MicrophoneRouting.isMicrophone(d)) continue;
            names.add(d.getProductName() + " · " + deviceType(d.getType()) + " " + d.getAddress()); deviceIds.add(d.getId());
        }
        input.setAdapter(adapter(names.toArray(new String[0])));
        int index = deviceIds.indexOf(prefs.getInt("device", 0)); input.setSelection(Math.max(0, index));
    }
    private String deviceType(int type) {
        return switch (type) { case AudioDeviceInfo.TYPE_BUILTIN_MIC -> "internes Mikrofon"; case AudioDeviceInfo.TYPE_USB_DEVICE, AudioDeviceInfo.TYPE_USB_HEADSET -> "USB"; case AudioDeviceInfo.TYPE_WIRED_HEADSET -> "Headset"; case AudioDeviceInfo.TYPE_BLUETOOTH_SCO -> "Bluetooth"; default -> "Audioeingang"; };
    }
    private void updateChannelNames() {
        updating = true; displayedChannels = State.channelNames.clone();
        left.setAdapter(adapter(displayedChannels)); right.setAdapter(adapter(displayedChannels));
        left.setSelection(State.left); right.setSelection(1 - State.left); updating = false;
    }
    private AdapterView.OnItemSelectedListener channelListener(boolean isLeft) {
        return new AdapterView.OnItemSelectedListener() {
            public void onItemSelected(AdapterView<?> parent, View view, int pos, long id) {
                if (updating || pos < 0 || pos > 1 || pos != parent.getSelectedItemPosition()) return;
                // Ignore adapter refresh callbacks; they are not user selections.
                if (pos == (isLeft ? State.left : State.right)) return;
                updating = true;
                State.left = isLeft ? pos : 1 - pos; State.right = 1 - State.left;
                left.setSelection(State.left); right.setSelection(State.right);
                prefs.edit().putInt("left", State.left).putInt("right", State.right).apply();
                updating = false;
            }
            public void onNothingSelected(AdapterView<?> parent) { }
        };
    }
    private ArrayAdapter<String> adapter(String[] items) { ArrayAdapter<String> a = new ArrayAdapter<>(this, android.R.layout.simple_spinner_item, items); a.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item); return a; }
    private String addresses() {
        List<String> values = new ArrayList<>();
        try {
            for (NetworkInterface iface : Collections.list(NetworkInterface.getNetworkInterfaces())) {
                if (!iface.isUp() || iface.isLoopback()) continue;
                for (InetAddress address : Collections.list(iface.getInetAddresses()))
                    if (address instanceof Inet4Address && address.isSiteLocalAddress()) values.add(address.getHostAddress());
            }
        } catch (Exception ignored) { }
        return values.isEmpty() ? "Keine lokale IPv4 · WLAN verbinden" : String.join("\n", values);
    }
    private TextView label(String text, int size, int color) { TextView v = new TextView(this); v.setText(text); v.setTextSize(size); v.setTextColor(color); v.setPadding(0,dp(9),0,dp(9)); add(v); return v; }
    private Button button(String text, View.OnClickListener listener) { Button b = new Button(this); b.setText(text); b.setAllCaps(false); b.setOnClickListener(listener); add(b); return b; }
    private void add(View view) { content.addView(view, new LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)); }
    private int dp(int n) { return Math.round(n * getResources().getDisplayMetrics().density); }
    @Override public void onResume() { super.onResume(); handler.post(tick); }
    @Override public void onPause() { handler.removeCallbacks(tick); super.onPause(); }
}

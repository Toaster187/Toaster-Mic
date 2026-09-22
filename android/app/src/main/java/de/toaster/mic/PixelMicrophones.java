package de.toaster.mic;

import android.content.SharedPreferences;
import android.media.*;
import android.os.Build;

final class PixelMicrophones {
    private PixelMicrophones() { }
    static boolean supported() { return "Google".equalsIgnoreCase(Build.MANUFACTURER) && "tokay".equals(Build.DEVICE); }
    static boolean calibrated(SharedPreferences prefs) {
        return Build.FINGERPRINT.equals(prefs.getString("pixelMapBuild", "")) &&
            ChannelCalibration.distinct(prefs.getInt("pixelTop", -1), prefs.getInt("pixelBottom", -1), prefs.getInt("pixelRear", -1));
    }
    static AudioDeviceInfo input(AudioManager manager) {
        for (AudioDeviceInfo device : manager.getDevices(AudioManager.GET_DEVICES_INPUTS))
            if (device.getType() == AudioDeviceInfo.TYPE_BUILTIN_MIC && "bottom".equals(device.getAddress())) return device;
        throw new IllegalStateException("Pixel-Rohmikrofone nicht verfügbar.");
    }
    static AudioFormat format() {
        return new AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT).setSampleRate(Wire.RATE).setChannelIndexMask(7).build();
    }
}

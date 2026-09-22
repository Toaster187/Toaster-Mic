package de.toaster.mic;

import android.media.*;
import android.util.Pair;
import java.util.*;

/** Physical identities come from the active HAL channel mapping, not device count. */
final class MicrophoneRouting {
    private MicrophoneRouting() { }
    static boolean isMicrophone(AudioDeviceInfo device) {
        int type = device.getType();
        return type == AudioDeviceInfo.TYPE_BUILTIN_MIC || type == AudioDeviceInfo.TYPE_WIRED_HEADSET ||
            type == AudioDeviceInfo.TYPE_USB_DEVICE || type == AudioDeviceInfo.TYPE_USB_ACCESSORY ||
            type == AudioDeviceInfo.TYPE_USB_HEADSET || type == AudioDeviceInfo.TYPE_BLUETOOTH_SCO ||
            (android.os.Build.VERSION.SDK_INT >= 31 && type == AudioDeviceInfo.TYPE_BLE_HEADSET);
    }
    static String[] distinctDirectPair(List<MicrophoneInfo> microphones) {
        String[] identities = new String[2], names = new String[2];
        boolean[] mixed = new boolean[2];
        for (MicrophoneInfo mic : microphones) {
            String identity = mic.getDescription() + "@" + mic.getAddress() + "#" + mic.getId();
            for (Pair<Integer, Integer> mapping : mic.getChannelMapping()) {
                int channel = mapping.first;
                if (channel < 0 || channel > 1) continue;
                if (mapping.second != MicrophoneInfo.CHANNEL_MAPPING_DIRECT ||
                    identities[channel] != null && !identities[channel].equals(identity)) mixed[channel] = true;
                identities[channel] = identity;
                names[channel] = switch (mic.getAddress()) {
                    case "bottom" -> "Mikrofon unten";
                    case "back" -> "Mikrofon hinten";
                    default -> mic.getDescription() + " · #" + mic.getId();
                };
            }
        }
        return !mixed[0] && !mixed[1] && identities[0] != null && identities[1] != null &&
            !identities[0].equals(identities[1]) ? names : null;
    }
}

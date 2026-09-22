package de.toaster.mic;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.security.MessageDigest;
import java.util.Arrays;
import javax.crypto.Mac;
import javax.crypto.Cipher;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;

public final class Wire {
    public static final int RATE = 48000, FRAMES = 240, HEADER = 36, TAG = 16;
    public static final int HELLO = 1, AUDIO = 2, STOP = 3;
    private final Mac mac;
    private final SecretKeySpec audioKey;
    public Wire(byte[] key) throws Exception {
        if (key.length != 32) throw new IllegalArgumentException("Expected AES-256 key");
        audioKey = new SecretKeySpec(key, "AES");
        mac = Mac.getInstance("HmacSHA256");
        mac.init(new SecretKeySpec(key, "HmacSHA256"));
    }
    public byte[] encode(int type, int channels, boolean muted, long session, int sequence, byte[] audio) throws Exception {
        byte[] result = new byte[HEADER + audio.length + TAG];
        ByteBuffer b = ByteBuffer.wrap(result).order(ByteOrder.LITTLE_ENDIAN);
        b.put(new byte[]{'T','M','I','C'}).put((byte)2).put((byte)type).put((byte)channels).put((byte)(muted ? 1 : 0));
        b.putLong(session).putInt(sequence).putInt(RATE).putShort((short)(type == AUDIO ? FRAMES : 0)).putShort((short)0).putLong(0).put(audio);
        if (type == AUDIO) {
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.ENCRYPT_MODE, audioKey, new GCMParameterSpec(128, Arrays.copyOfRange(result, 8, 20)));
            cipher.updateAAD(result, 0, HEADER);
            byte[] encrypted = cipher.doFinal(audio);
            System.arraycopy(encrypted, 0, result, HEADER, encrypted.length);
        } else {
            mac.update(result, 0, result.length - TAG);
            System.arraycopy(mac.doFinal(), 0, result, result.length - TAG, TAG);
        }
        return result;
    }
    public Control control(byte[] data, int length) {
        if (length != HEADER + TAG || data.length < length || data[0] != 'T' || data[1] != 'M' || data[2] != 'I' || data[3] != 'C' || data[4] != 2 || data[6] != 0) return null;
        if (data[5] != HELLO && data[5] != STOP) return null;
        ByteBuffer b = ByteBuffer.wrap(data, 0, length).order(ByteOrder.LITTLE_ENDIAN);
        if (b.getInt(20) != RATE || b.getShort(24) != 0) return null;
        mac.update(data, 0, length - TAG);
        if (!MessageDigest.isEqual(Arrays.copyOf(mac.doFinal(), TAG), Arrays.copyOfRange(data, length - TAG, length))) return null;
        return new Control(data[5], b.getLong(8));
    }
    public record Control(int type, long session) { }
}

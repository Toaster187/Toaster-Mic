package de.toaster.mic;

import java.math.BigInteger;
import java.nio.*;
import java.security.*;
import java.security.interfaces.ECPublicKey;
import java.security.spec.*;
import java.util.Arrays;
import javax.crypto.KeyAgreement;

public final class KeyExchange {
    private final KeyPair pair;
    public KeyExchange() throws GeneralSecurityException {
        KeyPairGenerator generator = KeyPairGenerator.getInstance("EC");
        generator.initialize(new ECGenParameterSpec("secp256r1"));
        pair = generator.generateKeyPair();
    }
    public byte[] offer(int type, long session) {
        byte[] data = new byte[80];
        ByteBuffer b = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);
        b.put(new byte[]{'T','M','K','2'}).put((byte)type); b.position(8); b.putLong(session);
        ECPoint point = ((ECPublicKey)pair.getPublic()).getW();
        coordinate(point.getAffineX(), data, 16); coordinate(point.getAffineY(), data, 48);
        return data;
    }
    private static void coordinate(BigInteger value, byte[] data, int offset) {
        byte[] bytes = value.toByteArray(); int count = Math.min(32, bytes.length);
        System.arraycopy(bytes, bytes.length - count, data, offset + 32 - count, count);
    }
    public static boolean isOffer(byte[] data, int length, int type) {
        return length == 80 && data.length >= length && data[0]=='T' && data[1]=='M' && data[2]=='K' && data[3]=='2' &&
            data[4]==type && data[5]==0 && data[6]==0 && data[7]==0;
    }
    public static long session(byte[] data) { return ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN).getLong(8); }
    public byte[] derive(byte[] data) throws GeneralSecurityException {
        if (data.length != 80) throw new GeneralSecurityException("Invalid key exchange");
        ECPoint point = new ECPoint(new BigInteger(1, Arrays.copyOfRange(data,16,48)), new BigInteger(1, Arrays.copyOfRange(data,48,80)));
        PublicKey remote = KeyFactory.getInstance("EC").generatePublic(new ECPublicKeySpec(point, ((ECPublicKey)pair.getPublic()).getParams()));
        KeyAgreement agreement = KeyAgreement.getInstance("ECDH");
        agreement.init(pair.getPrivate()); agreement.doPhase(remote, true);
        return MessageDigest.getInstance("SHA-256").digest(agreement.generateSecret());
    }
}

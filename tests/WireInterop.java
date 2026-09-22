import de.toaster.mic.Wire;
import de.toaster.mic.KeyExchange;
import java.security.*;
import java.security.spec.ECGenParameterSpec;
import javax.crypto.KeyAgreement;
import java.security.interfaces.ECPublicKey;
import java.util.Arrays;
import java.nio.*;
import java.nio.file.*;
import java.util.HexFormat;

public final class WireInterop {
    public static void main(String[] args) throws Exception {
        Wire wire = new Wire(MessageDigest.getInstance("SHA-256").digest("interop-test-key".getBytes(java.nio.charset.StandardCharsets.US_ASCII)));
        byte[] hello = wire.encode(Wire.HELLO, 0, false, 42, 0, new byte[0]);
        if (wire.control(hello, hello.length).session() != 42) throw new AssertionError("Control decode");
        hello[8] ^= 1;
        if (wire.control(hello, hello.length) != null) throw new AssertionError("Forged control accepted");
        hello[8] ^= 1;
        byte[] pcm = new byte[960]; ByteBuffer b = ByteBuffer.wrap(pcm).order(ByteOrder.LITTLE_ENDIAN);
        for (int i = 0; i < 240; i++) b.putShort((short)1000).putShort((short)-2000);
        byte[] audio = wire.encode(Wire.AUDIO, 2, false, 42, 7, pcm);
        KeyExchange a = new KeyExchange(), bKey = new KeyExchange();
        if (!Arrays.equals(a.derive(bKey.offer(2,42)), bKey.derive(a.offer(1,42)))) throw new AssertionError("ECDH differs");
        byte[] bad = bKey.offer(2,42); Arrays.fill(bad,16,80,(byte)0);
        boolean rejected = false; try { a.derive(bad); } catch (GeneralSecurityException e) { rejected = true; }
        if (!rejected) throw new AssertionError("Invalid point accepted");
        for (int n=0;n<80;n++) if (KeyExchange.isOffer(bad,n,2)) throw new AssertionError("Truncated exchange");
        // A fresh test-only key allows the C# suite to independently verify Java's KDF.
        KeyPairGenerator gen = KeyPairGenerator.getInstance("EC"); gen.initialize(new ECGenParameterSpec("secp256r1"));
        KeyPair test = gen.generateKeyPair();
        byte[] offer = a.offer(1,42);
        ECPublicKey pub = (ECPublicKey)test.getPublic();
        byte[][] coords = {pub.getW().getAffineX().toByteArray(),pub.getW().getAffineY().toByteArray()};
        for (int i=0;i<2;i++) { Arrays.fill(offer,16+i*32,48+i*32,(byte)0); int n=Math.min(32,coords[i].length); System.arraycopy(coords[i],coords[i].length-n,offer,16+i*32+32-n,n); }
        var hex = HexFormat.of().withUpperCase();
        Files.writeString(Path.of(args[0]), hex.formatHex(hello) + "\n" + hex.formatHex(audio) + "\n" +
            hex.formatHex(test.getPrivate().getEncoded()) + "\n" + hex.formatHex(bKey.offer(2,42)) + "\n" + hex.formatHex(bKey.derive(offer)) + "\n");
        System.out.println("Java wire checks passed; interoperability vectors written.");
    }
}

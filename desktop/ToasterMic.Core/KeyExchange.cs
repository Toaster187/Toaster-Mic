using System.Buffers.Binary;
using System.Security.Cryptography;

namespace ToasterMic.Core;

// Ephemeral P-256 exchange: confidentiality without a manually entered secret.
// It does not authenticate the other device against an active LAN attacker.
public sealed class KeyExchange : IDisposable
{
    private readonly ECDiffieHellman ec = ECDiffieHellman.Create(ECCurve.NamedCurves.nistP256);
    public byte[] Offer(byte type, long session)
    {
        byte[] data = new byte[80]; "TMK2"u8.CopyTo(data); data[4] = type;
        BinaryPrimitives.WriteInt64LittleEndian(data.AsSpan(8), session);
        var p = ec.ExportParameters(false);
        p.Q.X!.CopyTo(data, 16); p.Q.Y!.CopyTo(data, 48);
        return data;
    }
    public static bool IsOffer(ReadOnlySpan<byte> data, byte type, long session) =>
        data.Length == 80 && data[..4].SequenceEqual("TMK2"u8) && data[4] == type &&
        data[5] == 0 && data[6] == 0 && data[7] == 0 && BinaryPrimitives.ReadInt64LittleEndian(data[8..]) == session;
    public byte[] Derive(ReadOnlySpan<byte> offer)
    {
        if (offer.Length != 80) throw new CryptographicException("Ungültiger Schlüsselaustausch.");
        try {
            using var remote = ECDiffieHellman.Create(new ECParameters {
                Curve = ECCurve.NamedCurves.nistP256,
                Q = new ECPoint { X = offer.Slice(16, 32).ToArray(), Y = offer.Slice(48, 32).ToArray() }
            });
            return ec.DeriveKeyFromHash(remote.PublicKey, HashAlgorithmName.SHA256);
        }
        catch (PlatformNotSupportedException e) { throw new CryptographicException("Ungültiger P-256-Schlüssel.", e); }
    }
    public void Dispose() => ec.Dispose();
}

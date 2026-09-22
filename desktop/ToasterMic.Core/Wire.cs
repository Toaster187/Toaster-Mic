using System.Buffers.Binary;
using System.Security.Cryptography;

namespace ToasterMic.Core;

public sealed record Packet(byte Type, byte Channels, byte Flags, long Session, uint Sequence, byte[] Audio);

public static class Wire
{
    public const int Rate = 48000, Frames = 240, Header = 36, Tag = 16;
    public const byte Hello = 1, Audio = 2, Stop = 3;
    public static byte[] Encode(byte type, byte channels, byte flags, long session, uint sequence, ReadOnlySpan<byte> audio, byte[] key)
    {
        var data = new byte[Header + audio.Length + Tag];
        "TMIC"u8.CopyTo(data); data[4] = 2; data[5] = type; data[6] = channels; data[7] = flags;
        BinaryPrimitives.WriteInt64LittleEndian(data.AsSpan(8), session);
        BinaryPrimitives.WriteUInt32LittleEndian(data.AsSpan(16), sequence);
        BinaryPrimitives.WriteInt32LittleEndian(data.AsSpan(20), Rate);
        BinaryPrimitives.WriteUInt16LittleEndian(data.AsSpan(24), type == Audio ? (ushort)Frames : (ushort)0);
        if (type == Audio)
        {
            using var aes = new AesGcm(key, Tag);
            aes.Encrypt(data.AsSpan(8, 12), audio, data.AsSpan(Header, audio.Length), data.AsSpan(data.Length - Tag), data.AsSpan(0, Header));
        }
        else HMACSHA256.HashData(key, data.AsSpan(0, data.Length - Tag)).AsSpan(0, Tag).CopyTo(data.AsSpan(data.Length - Tag));
        return data;
    }
    public static Packet? Decode(ReadOnlySpan<byte> data, byte[] key)
    {
        if (data.Length < Header + Tag || !data[..4].SequenceEqual("TMIC"u8) || data[4] != 2) return null;
        byte type = data[5], channels = data[6];
        if (type is not (Hello or Audio or Stop) || BinaryPrimitives.ReadInt32LittleEndian(data[20..]) != Rate) return null;
        int frames = BinaryPrimitives.ReadUInt16LittleEndian(data[24..]);
        if (type == Audio ? channels is not (1 or 2) || frames != Frames : channels != 0 || frames != 0) return null;
        if (data.Length != Header + frames * channels * 2 + Tag) return null;
        var audio = new byte[frames * channels * 2];
        if (type == Audio)
        {
            try
            {
                using var aes = new AesGcm(key, Tag);
                aes.Decrypt(data.Slice(8, 12), data.Slice(Header, audio.Length), data[^Tag..], audio, data[..Header]);
            }
            catch (CryptographicException) { return null; }
        }
        else
        {
            var tag = HMACSHA256.HashData(key, data[..^Tag]);
            if (!CryptographicOperations.FixedTimeEquals(tag.AsSpan(0, Tag), data[^Tag..])) return null;
        }
        return new(type, channels, data[7], BinaryPrimitives.ReadInt64LittleEndian(data[8..]),
            BinaryPrimitives.ReadUInt32LittleEndian(data[16..]), audio);
    }
}

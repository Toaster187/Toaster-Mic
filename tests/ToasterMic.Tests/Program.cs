using System.Buffers.Binary;
using System.Net;
using System.Net.Sockets;
using System.Security.Cryptography;
using ToasterMic.Core;

byte[] key = SHA256.HashData("interop-test-key"u8);
int passed = 0;
void Check(bool condition, string message) { if (!condition) throw new Exception(message); passed++; Console.WriteLine("PASS " + message); }
Packet P(uint seq, short l = 1000, short r = -2000, byte channels = 2)
{
    byte[] pcm = new byte[Wire.Frames * channels * 2];
    for (int i = 0; i < Wire.Frames; i++) { BinaryPrimitives.WriteInt16LittleEndian(pcm.AsSpan(i * channels * 2), l); if (channels == 2) BinaryPrimitives.WriteInt16LittleEndian(pcm.AsSpan(i * 4 + 2), r); }
    return new(Wire.Audio, channels, 0, 42, seq, pcm);
}
byte[] Read(JitterBuffer buffer) { byte[] bytes = new byte[Wire.Frames * 4]; buffer.Read(bytes); return bytes; }
short Sample(byte[] b, int index = 0) => BinaryPrimitives.ReadInt16LittleEndian(b.AsSpan(index * 2));
var source = P(7);
var encoded = Wire.Encode(source.Type, source.Channels, source.Flags, source.Session, source.Sequence, source.Audio, key);
var decoded = Wire.Decode(encoded, key);
Check(decoded != null && decoded.Audio.SequenceEqual(source.Audio) && decoded.Sequence == 7 && decoded.Session == 42, "encrypted stereo roundtrip");
Check(encoded.Length == 1012, "stereo fits a normal WLAN MTU without IP fragmentation");
Check(!encoded.AsSpan(Wire.Header, source.Audio.Length).SequenceEqual(source.Audio), "audio is encrypted on the wire");
for (int i = 0; i < encoded.Length; i++) { encoded[i] ^= 1; if (Wire.Decode(encoded, key) != null) throw new Exception("Tamper accepted at " + i); encoded[i] ^= 1; }
Check(true, "every single-byte tamper rejected");
Check(Wire.Decode(encoded, new byte[32]) == null, "different session key rejected");
for (int n = 0; n < encoded.Length; n++) if (Wire.Decode(encoded.AsSpan(0, n), key) != null) throw new Exception("Truncation accepted");
Check(true, "all truncated packets rejected without exceptions");
Check(Wire.Decode([..encoded, 0], key) == null, "trailing bytes rejected");
foreach (byte type in new[]{Wire.Hello, Wire.Stop}) Check(Wire.Decode(Wire.Encode(type, 0, 0, 42, 0, [], key), key)?.Type == type, "authenticated control " + type);
using (var pc = new KeyExchange())
using (var phone = new KeyExchange())
{
    var request = pc.Offer(1, 42); var reply = phone.Offer(2, 42);
    Check(KeyExchange.IsOffer(request, 1, 42) && KeyExchange.IsOffer(reply, 2, 42), "code-free handshake framing");
    Check(pc.Derive(reply).SequenceEqual(phone.Derive(request)), "automatic P-256 session key agreement");
    Check(!KeyExchange.IsOffer(reply, 2, 43) && !KeyExchange.IsOffer(reply, 1, 42), "wrong-session and wrong-direction exchange rejected");
    for (int n = 0; n < 80; n++) if (KeyExchange.IsOffer(reply.AsSpan(0, n), 2, 42)) throw new Exception("Short exchange accepted");
    Check(!KeyExchange.IsOffer([..reply, 0], 2, 42), "truncated and oversized exchanges rejected");
    var invalid = reply.ToArray(); Array.Clear(invalid, 16, 64); bool rejected = false;
    try { pc.Derive(invalid); } catch (CryptographicException) { rejected = true; }
    Check(rejected, "invalid curve point rejected");
    using var restarted = new KeyExchange();
    Check(!pc.Derive(reply).SequenceEqual(pc.Derive(restarted.Offer(2, 42))), "phone restart negotiates a fresh key");
    Check(pc.Derive(reply).SequenceEqual(pc.Derive(reply)), "retried handshake derives the same session key");
}
var rng = new Random(1);
for (int i = 0; i < 10000; i++) { byte[] noise = new byte[rng.Next(0, 1600)]; rng.NextBytes(noise); Wire.Decode(noise, key); }
Check(true, "10,000 malformed datagrams handled");
var reorder = new JitterBuffer(3); reorder.Push(P(10)); reorder.Push(P(12, 1200));
Check(Read(reorder).All(x => x == 0), "waits for startup buffer");
reorder.Push(P(11, 1100));
Check(Sample(Read(reorder)) == 1000 && Sample(Read(reorder)) == 1100 && Sample(Read(reorder)) == 1200, "out-of-order delivery restored");
var lost = new JitterBuffer(1); lost.Push(P(1)); lost.Push(P(3, 3000)); Read(lost);
Check(Read(lost).All(x => x == 0) && Sample(Read(lost)) == 3000 && lost.Stats.Lost == 1, "packet loss concealed without blocking future audio");
lost.Push(P(2)); Check(lost.Stats.Late == 1, "late packets discarded");
var mono = new JitterBuffer(1); mono.Push(P(1, 2345, channels: 1)); var mb = Read(mono);
Check(Sample(mb) == 2345 && Sample(mb, 1) == 2345, "mono duplicated to both desktop output channels");
var stereo = new JitterBuffer(1); stereo.Push(P(1)); var sb = Read(stereo);
Check(Sample(sb) == 1000 && Sample(sb, 1) == -2000, "left and right stay independent");
var volume = new JitterBuffer(1) { Gain = 2 }; volume.Push(P(1, 25000, -25000)); var vb = Read(volume);
Check(Sample(vb) == 32767 && Sample(vb, 1) == -32768, "gain clips safely without signed overflow");
volume.Muted = true; volume.Push(P(2)); Check(Read(volume).All(x => x == 0), "mute emits silence");
volume.Muted = false; volume.Push(P(3, 1234));
Check(Sample(Read(volume)) == 2468, "unmute resumes current audio instead of replaying muted audio");
var boost = new JitterBuffer(1) { Gain = 10 }; boost.Push(P(1, 1000, -1000));
var boosted = Read(boost);
Check(Sample(boosted) == 10000 && Sample(boosted, 1) == -10000, "1000 percent applies a real 20 dB boost");
var adaptive = new JitterBuffer(1, adaptive: true);
byte[] period = new byte[Wire.Frames * 4 * 2];
adaptive.Push(P(0)); adaptive.Read(period);
Check(period.All(x => x == 0) && adaptive.TargetMs == 15, "5 ms preset reserves enough audio for a 10 ms device callback");
adaptive.Push(P(1)); adaptive.Push(P(2)); adaptive.Read(period);
Check(Sample(period) == 1000 && adaptive.Stats.Lost == 0, "device callback plays without a built-in underrun");
Read(adaptive); var fade = Read(adaptive);
Check(Sample(fade) > 0 && Sample(fade, Wire.Frames * 2 - 2) == 0 && adaptive.TargetMs == 20, "underrun fades to silence and raises the reserve");
for (int i = 0; i < 6; i++) Read(adaptive);
for (uint i = 3; i < 7; i++) adaptive.Push(P(i, 2000));
var recovered = Read(adaptive);
Check(Sample(recovered, 100) == 2000 && adaptive.Stats.Late == 0, "late burst recovers without permanently chasing packet sequence");
adaptive.Reset();
Check(adaptive.TargetMs == 5 && Read(adaptive).All(x => x == 0), "new session clears the learned network reserve");
var cadence = new JitterBuffer(1, adaptive: true);
uint sent = 0;
// Android often delivers two 5-ms packets in one capture burst.
for (int i = 0; i < 1000; i++)
{
    cadence.Push(P(sent++)); cadence.Push(P(sent++)); cadence.Read(period);
}
Check(cadence.Stats.Lost == 0 && cadence.TargetMs == 15, "1000 batched capture/device periods do not stutter at the lowest preset");
for (int i = 0; i < 100; i++) { for (int j = 0; j < 14; j++) Read(cadence); for (int j = 0; j < 12; j++) cadence.Push(P(sent++)); }
Check(cadence.TargetMs <= 60 && cadence.Stats.BufferedMs <= 95, "repeated network stalls keep adaptation and latency bounded");
var quietRecovery = new JitterBuffer(1, adaptive: true);
uint quietSeq = 0;
// Inflate the reserve with a large device startup read, then return to 10 ms.
quietRecovery.Read(new byte[Wire.Frames * 4 * 10]);
for (int i = 0; i < 11; i++) quietRecovery.Push(P(quietSeq++, 0, 0));
for (int i = 0; i < 1200; i++)
{
    quietRecovery.Push(P(quietSeq++, 0, 0)); quietRecovery.Push(P(quietSeq++, 0, 0)); quietRecovery.Read(period);
}
Check(quietRecovery.TargetMs == 15 && quietRecovery.Stats.BufferedMs <= 15 && quietRecovery.TrimmedPackets > 0,
    "speech pauses reduce learned reserve and queued latency without reconnecting");
var speechRecovery = new JitterBuffer(1, adaptive: true) { Muted = true };
uint speechSeq = 0;
speechRecovery.Read(new byte[Wire.Frames * 4 * 10]);
for (int i = 0; i < 11; i++) speechRecovery.Push(P(speechSeq++));
for (int i = 0; i < 1200; i++)
{
    speechRecovery.Push(P(speechSeq++)); speechRecovery.Push(P(speechSeq++)); speechRecovery.Read(period);
}
Check(speechRecovery.TargetMs == 55 && speechRecovery.TrimmedPackets == 0,
    "continuous speech is never trimmed, even when output is muted");
var protectedSpeech = new JitterBuffer(1, adaptive: true);
uint protectedSeq = 0;
for (int i = 0; i < 3; i++) protectedSpeech.Push(P(protectedSeq++, 0, 0));
for (int i = 0; i < 200; i++)
{
    protectedSpeech.Push(P(protectedSeq++, 0, 0)); protectedSpeech.Push(P(protectedSeq++, 0, 0)); protectedSpeech.Read(period);
}
for (int i = 0; i < 5; i++) protectedSpeech.Push(P(protectedSeq++, 2345, -2345));
int audiblePackets = 0;
for (int i = 0; i < 10; i++) if (Sample(Read(protectedSpeech), 100) == 2345) audiblePackets++;
Check(audiblePackets == 5, "quiet-prefix trim preserves all queued speech packets");
var boostedQuiet = new JitterBuffer(1, adaptive: true) { Gain = 10 };
uint boostedSeq = 0; boostedQuiet.Read(new byte[Wire.Frames * 4 * 10]);
for (int i = 0; i < 11; i++) boostedQuiet.Push(P(boostedSeq++, 50, -50));
for (int i = 0; i < 600; i++) { boostedQuiet.Push(P(boostedSeq++, 50, -50)); boostedQuiet.Push(P(boostedSeq++, 50, -50)); boostedQuiet.Read(period); }
Check(boostedQuiet.TrimmedPackets == 0, "quiet detection accounts for amplified low-level speech");
var selectedReserve = new JitterBuffer(12, adaptive: true);
uint reservedSeq = 0;
for (int i = 0; i < 12; i++) selectedReserve.Push(P(reservedSeq++, 0, 0));
for (int i = 0; i < 600; i++) { selectedReserve.Push(P(reservedSeq++, 0, 0)); selectedReserve.Push(P(reservedSeq++, 0, 0)); selectedReserve.Read(period); }
Check(selectedReserve.TargetMs == 60, "quiet recovery respects the user's configured minimum reserve");
var bounded = new JitterBuffer(3); for (uint i = 0; i < 1000; i++) bounded.Push(P(i));
Check(bounded.Stats.BufferedMs <= 65 && bounded.Stats.Resyncs > 0, "consumer slowdown cannot grow latency without bound");
var wrap = new JitterBuffer(1); wrap.Push(P(uint.MaxValue)); wrap.Push(P(0, 2000));
Check(Sample(Read(wrap)) == 1000 && Sample(Read(wrap)) == 2000, "sequence wrap handled");
var drought = new JitterBuffer(1); drought.Push(P(0)); Read(drought); for (int i = 0; i < 20; i++) Read(drought);
drought.Push(P(500, 5000)); Check(Sample(Read(drought)) == 5000, "restarts after prolonged network dropout");
var duplicate = new JitterBuffer(1); duplicate.Push(P(1)); duplicate.Push(P(1)); Check(duplicate.Stats.Received == 1, "duplicate datagram ignored");
var partial = new JitterBuffer(1); partial.Push(P(1)); byte[] half = new byte[480]; partial.Read(half); partial.Read(half); Check(Sample(half) == 1000, "audio device reads may split a network packet");
partial.Reset(); Check(Read(partial).All(x => x == 0), "rekey clears buffered audio");
partial.Push(P(0, 321)); Check(Sample(Read(partial)) == 321, "new recording can restart packet sequence");
var concurrent = new JitterBuffer(3);
await Task.WhenAll(Task.Run(() => { for (uint i = 0; i < 10000; i++) concurrent.Push(P(i)); }), Task.Run(() => { for (int i = 0; i < 10000; i++) Read(concurrent); }));
Check(concurrent.Stats.BufferedMs <= 65, "concurrent network producer and audio consumer");
using (var server = new UdpClient(new IPEndPoint(IPAddress.Loopback, 0)))
using (var client = new UdpClient())
{
    client.Connect((IPEndPoint)server.Client.LocalEndPoint!);
    using var timeout = new CancellationTokenSource(3000);
    using var pc = new KeyExchange(); using var phone = new KeyExchange();
    await client.SendAsync(pc.Offer(1, 42));
    var offer = await server.ReceiveAsync(timeout.Token);
    Check(KeyExchange.IsOffer(offer.Buffer, 1, 42), "UDP loopback no-code offer");
    byte[] phoneKey = phone.Derive(offer.Buffer);
    // Drop the first reply, then retry as the apps do after packet loss.
    await client.SendAsync(pc.Offer(1, 42));
    var retry = await server.ReceiveAsync(timeout.Token);
    Check(retry.Buffer.SequenceEqual(offer.Buffer), "UDP loopback lost handshake reply can be retried");
    await server.SendAsync(phone.Offer(2, 42), retry.RemoteEndPoint);
    var reply = await client.ReceiveAsync(timeout.Token);
    Check(KeyExchange.IsOffer(reply.Buffer, 2, 42), "UDP loopback no-code reply");
    byte[] pcKey = pc.Derive(reply.Buffer);
    Check(pcKey.SequenceEqual(phoneKey), "UDP loopback automatic session key");
    await client.SendAsync(Wire.Encode(Wire.Hello, 0, 0, 42, 0, [], pcKey));
    var hello = await server.ReceiveAsync(timeout.Token);
    Check(Wire.Decode(hello.Buffer, phoneKey)?.Session == 42, "UDP loopback authenticated handshake");
    await server.SendAsync(Wire.Encode(Wire.Audio, 2, 0, 42, 7, source.Audio, phoneKey), hello.RemoteEndPoint);
    var audio = await client.ReceiveAsync(timeout.Token);
    Check(Wire.Decode(audio.Buffer, pcKey)?.Audio.SequenceEqual(source.Audio) == true, "UDP loopback encrypted audio reception");
}
if (args.Length == 1)
{
    string[] vectors = File.ReadAllLines(args[0]);
    Check(Convert.ToHexString(Wire.Encode(Wire.Hello, 0, 0, 42, 0, [], key)) == vectors[0], "Java/C# identical HELLO bytes");
    Check(Convert.ToHexString(encoded) == vectors[1], "Java/C# identical AES-GCM stereo packet");
    // Independent providers: Java emits a test-only private key and the peer's public offer.
    using var client = ECDiffieHellman.Create();
    client.ImportPkcs8PrivateKey(Convert.FromHexString(vectors[2]), out _);
    byte[] reply = Convert.FromHexString(vectors[3]);
    using var peer = ECDiffieHellman.Create(new ECParameters { Curve = ECCurve.NamedCurves.nistP256,
        Q = new ECPoint { X = reply[16..48], Y = reply[48..80] } });
    Check(KeyExchange.IsOffer(reply, 2, 42), "Java/C# identical key exchange format");
    Check(Convert.ToHexString(client.DeriveKeyFromHash(peer.PublicKey, HashAlgorithmName.SHA256)) == vectors[4], "Java/C# ECDH produces identical AES key");
}
await BridgeTests.Run(Check);
Console.WriteLine($"{passed} checks passed.");

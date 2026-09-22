using System.Buffers.Binary;

namespace ToasterMic.Core;

// Fixed 5 ms packets. Bounded latency: reorder inside the window, conceal missing packets,
// and discard old audio when the consumer falls behind. All state is guarded by gate.
public sealed class JitterBuffer(int targetPackets, bool adaptive = false)
{
    private readonly object gate = new();
    private readonly Dictionary<uint, Packet> packets = new();
    private uint next;
    private bool initialized, playing;
    private int silentRun, position = Wire.Frames * 2;
    private readonly short[] current = new short[Wire.Frames * 2];
    private long lost, late, received, resyncs;
    private int peak;
    private int effectiveTarget = Math.Clamp(targetPackets, 1, 12);
    private int deviceFloor = 1, observedCallback, callbackFrames;
    private int quietPackets, stablePackets, recoveryCooldown;
    private long trimmed;
    private bool recovering;
    public int TargetMs { get { lock (gate) return effectiveTarget * 5; } }
    public long TrimmedPackets { get { lock (gate) return trimmed; } }
    public float Gain { get; set; } = 1;
    public bool Muted { get; set; }
    public void Reset()
    {
        lock (gate) { packets.Clear(); Array.Clear(current); initialized = playing = recovering = false; position = current.Length; silentRun = peak = quietPackets = stablePackets = recoveryCooldown = 0; deviceFloor = 1; observedCallback = callbackFrames = 0; effectiveTarget = Math.Clamp(targetPackets, 1, 12); }
    }
    public (long Received, long Lost, long Late, long Resyncs, int BufferedMs, int Peak) Stats
    { get { lock (gate) return (received, lost, late, resyncs, packets.Count * 5, peak); } }
    public void Push(Packet packet)
    {
        if (packet.Type != Wire.Audio) return;
        lock (gate)
        {
            if (!initialized) { next = packet.Sequence; initialized = true; }
            int distance = unchecked((int)(packet.Sequence - next));
            if (distance < 0) { late++; return; }
            if (distance > Math.Max(12, effectiveTarget + 6))
            {
                packets.Clear(); next = packet.Sequence; playing = false;
                position = current.Length; quietPackets = stablePackets = 0; resyncs++;
            }
            if (packets.TryAdd(packet.Sequence, packet)) received++;
        }
    }
    public void Read(Span<byte> destination)
    {
        if (destination.Length % 4 != 0) throw new ArgumentException("Stereo PCM requires whole frames.");
        lock (gate)
        {
            // A large startup callback must not permanently set the latency floor.
            // Retain the largest device period from the current one-second window.
            if (adaptive && destination.Length > 0)
            {
                int requested = Math.Clamp((destination.Length + Wire.Frames * 4 - 1) / (Wire.Frames * 4) + 1, 1, 12);
                observedCallback = Math.Max(observedCallback, requested);
                deviceFloor = Math.Max(deviceFloor, requested);
                callbackFrames += destination.Length / 4;
                if (callbackFrames >= 48000) { deviceFloor = observedCallback; observedCallback = 0; callbackFrames = 0; }
                effectiveTarget = Math.Max(effectiveTarget, deviceFloor);
            }
            peak = 0;
            float gain = float.IsFinite(Gain) ? Math.Clamp(Gain, 0, 10) : 1;
            bool muted = Muted;
            for (int offset = 0; offset < destination.Length; offset += 2)
            {
                if (position == current.Length) NextPacket();
                short input = current[position++];
                int sample = muted ? 0 : (int)Math.Clamp(input * gain, short.MinValue, short.MaxValue);
                peak = Math.Max(peak, Math.Abs(sample));
                BinaryPrimitives.WriteInt16LittleEndian(destination[offset..], (short)sample);
            }
        }
    }
    private void NextPacket()
    {
        short lastLeft = current[^2], lastRight = current[^1];
        position = 0; Array.Clear(current);
        if (!playing)
        {
            if (packets.Count < effectiveTarget) return;
            playing = true; silentRun = 0;
        }
        if (adaptive) RecoverDuringQuiet();
        if (packets.Remove(next, out var p))
        {
            next++;
            silentRun = 0;
            stablePackets = Math.Min(400, stablePackets + 1);
            quietPackets = IsQuiet(p) ? Math.Min(200, quietPackets + 1) : 0;
            if (recoveryCooldown > 0) recoveryCooldown--;
            for (int frame = 0; frame < Wire.Frames; frame++)
            {
                current[frame * 2] = BinaryPrimitives.ReadInt16LittleEndian(p.Audio.AsSpan(frame * p.Channels * 2));
                current[frame * 2 + 1] = p.Channels == 1 ? current[frame * 2] : BinaryPrimitives.ReadInt16LittleEndian(p.Audio.AsSpan(frame * 4 + 2));
                if (recovering && frame < 48)
                {
                    current[frame * 2] = (short)(current[frame * 2] * frame / 48);
                    current[frame * 2 + 1] = (short)(current[frame * 2 + 1] * frame / 48);
                }
            }
            recovering = false;
        }
        else
        {
            lost++;
            quietPackets = stablePackets = 0;
            if (adaptive)
            {
                // Fade to zero for a missing packet; do not chase the sender forever
                // by advancing its sequence while the entire receive queue is empty.
                for (int frame = 0; frame < Wire.Frames; frame++)
                {
                    current[frame * 2] = (short)(lastLeft * (Wire.Frames - frame - 1) / Wire.Frames);
                    current[frame * 2 + 1] = (short)(lastRight * (Wire.Frames - frame - 1) / Wire.Frames);
                }
                recovering = true;
                effectiveTarget = Math.Min(12, effectiveTarget + 1);
                if (packets.Count == 0) { playing = false; return; }
            }
            next++;
            if (++silentRun >= 10) { playing = initialized = false; packets.Clear(); }
        }
    }
    private void RecoverDuringQuiet()
    {
        // One second of real quiet audio, two seconds without an underrun. Never
        // mistake missing packets or the output mute switch for a speech pause.
        if (quietPackets < 200 || stablePackets < 400 || recoveryCooldown > 0) return;
        if (!packets.TryGetValue(next, out var upcoming) || !IsQuiet(upcoming)) return;
        int floor = Math.Max(Math.Clamp(targetPackets, 1, 12), deviceFloor);
        effectiveTarget = Math.Max(floor, effectiveTarget - 2);
        // Only discard a contiguous, verified quiet prefix. Keep queued speech,
        // sequence numbers, the session key and the network connection intact.
        int removed = 0;
        while (packets.Count > effectiveTarget && removed < 8 &&
            packets.TryGetValue(next, out var candidate) && IsQuiet(candidate))
        {
            packets.Remove(next++); trimmed++; removed++;
        }
        if (removed > 0) recovering = true;
        recoveryCooldown = 200;
    }
    private bool IsQuiet(Packet packet)
    {
        double gain = float.IsFinite(Gain) ? Math.Max(1, Math.Clamp(Gain, 0, 10)) : 1;
        double energy = 0;
        int samples = packet.Audio.Length / 2;
        for (int i = 0; i < packet.Audio.Length; i += 2)
        {
            double sample = BinaryPrimitives.ReadInt16LittleEndian(packet.Audio.AsSpan(i)) * gain;
            if (Math.Abs(sample) > 300) return false;
            energy += sample * sample;
        }
        return samples > 0 && energy <= samples * 10000.0;
    }
}

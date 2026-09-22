namespace ToasterMic.Core;

// Only accepted audio renews the lease. Opening the UI or sending HELLO does not.
public sealed class ConnectionLease(long timeoutTicks)
{
    private long last = -1;
    public void Observe(long now) => Interlocked.Exchange(ref last, now);
    public void Clear() => Interlocked.Exchange(ref last, -1);
    public bool IsActive(long now)
    {
        long seen = Interlocked.Read(ref last);
        return seen >= 0 && now >= seen && now - seen < timeoutTicks;
    }
}

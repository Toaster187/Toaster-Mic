package de.toaster.mic;

public final class State {
    private State() { }
    public static volatile boolean running, muted, stereoVerified;
    public static volatile float gain = 1;
    public static volatile int left = 0, right = 1, peak, leftPeak, rightPeak;
    public static volatile String status = "Bereit", microphones = "Kanalinformationen werden beim Start ermittelt.";
    public static volatile String[] channelNames = {"Kanal 1", "Kanal 2"};
}

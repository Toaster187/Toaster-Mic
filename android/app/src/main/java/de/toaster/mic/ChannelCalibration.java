package de.toaster.mic;

/** Conservative level comparison; no audio samples are persisted. */
public final class ChannelCalibration {
    private ChannelCalibration() { }
    public static int identify(double[] background, double[] signal) {
        if (background.length != 3 || signal.length != 3) throw new IllegalArgumentException("Three raw channels required");
        double best = 0, second = 0; int channel = -1;
        for (int i = 0; i < 3; i++) {
            if (!Double.isFinite(background[i]) || !Double.isFinite(signal[i]) || background[i] < 0 || signal[i] < 0) return -1;
            double level = Math.sqrt(Math.max(0, signal[i] * signal[i] - background[i] * background[i]));
            if (level > best) { second = best; best = level; channel = i; } else second = Math.max(second, level);
        }
        if (channel < 0 || best < Math.max(40, background[channel] * 3) || best < second * 1.6) return -1;
        return channel;
    }
    public static boolean distinct(int top, int bottom, int rear) {
        return top >= 0 && top < 3 && bottom >= 0 && bottom < 3 && rear >= 0 && rear < 3 && top != bottom && top != rear && bottom != rear;
    }
}

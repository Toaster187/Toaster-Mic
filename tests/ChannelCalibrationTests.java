import de.toaster.mic.ChannelCalibration;

public class ChannelCalibrationTests {
    private static void check(boolean value) { if (!value) throw new AssertionError(); }
    public static void main(String[] args) {
        double[] quiet = {10, 10, 10};
        for (int channel = 0; channel < 3; channel++) {
            double[] touch = {30, 30, 30}; touch[channel] = 500;
            check(ChannelCalibration.identify(quiet, touch) == channel);
        }
        check(ChannelCalibration.identify(quiet, new double[]{500, 450, 30}) == -1);
        check(ChannelCalibration.identify(quiet, quiet) == -1);
        check(ChannelCalibration.identify(quiet, new double[]{Double.NaN, 900, 20}) == -1);
        check(ChannelCalibration.identify(new double[]{500, 10, 10}, new double[]{510, 100, 20}) == -1);
        for (int top = -1; top <= 3; top++) for (int bottom = -1; bottom <= 3; bottom++) for (int rear = -1; rear <= 3; rear++)
            check(ChannelCalibration.distinct(top, bottom, rear) == (top >= 0 && top < 3 && bottom >= 0 && bottom < 3 && rear >= 0 && rear < 3 && top != bottom && top != rear && bottom != rear));
        System.out.println("Calibration tests passed: dominance, ambiguity, quiet, invalid data, all mappings.");
    }
}

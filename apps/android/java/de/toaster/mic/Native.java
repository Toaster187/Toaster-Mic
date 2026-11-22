package de.toaster.mic;

import android.content.Context;

/** JNI surface of libtoaster_engine.so (Rust). Loaded only in the :audio process. */
final class Native {
    static { System.loadLibrary("toaster_engine"); }
    private Native() { }
    static native int start(Context context, boolean standby);
    static native void stop();
    static native void setMuted(boolean muted);
    static native void setGain(int percent);
    static native void setLeft(int left);
    static native void setLive(boolean live);
    static native void networkChanged();
    static native boolean isRunning();
    /** Called by the engine from its control thread. */
    static void onEngineEvent(int event, int profile) { MicService.post(event, profile); }
}

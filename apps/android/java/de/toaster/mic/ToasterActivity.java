package de.toaster.mic;

import android.app.NativeActivity;

/** Hosts the Slint UI (libtoaster_ui.so). Permission results are polled by Rust. */
public class ToasterActivity extends NativeActivity {
    @Override public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(requestCode, permissions, results);
    }
}

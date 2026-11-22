package de.toaster.mic;

import android.content.Context;
import java.io.File;
import java.nio.file.Files;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** Read-only view of settings.json (owned by the Rust UI). */
final class Prefs {
    private Prefs() { }
    private static String json(Context c) {
        try { return new String(Files.readAllBytes(new File(c.getFilesDir(), "settings.json").toPath()), "UTF-8"); }
        catch (Exception e) { return ""; }
    }
    static boolean bool(Context c, String key) {
        return Pattern.compile("\"" + key + "\"\\s*:\\s*true").matcher(json(c)).find();
    }
    static int integer(Context c, String key, int fallback) {
        Matcher m = Pattern.compile("\"" + key + "\"\\s*:\\s*(\\d+)").matcher(json(c));
        return m.find() ? Integer.parseInt(m.group(1)) : fallback;
    }
}

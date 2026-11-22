package de.toaster.mic;

import android.app.AutomaticZenRule;
import android.app.NotificationManager;
import android.content.ComponentName;
import android.content.Context;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Build;
import android.service.notification.Condition;
import android.service.notification.ZenPolicy;

/**
 * "Live": own do-not-disturb rule while transmitting. Notifications and their
 * vibration are silenced; media, alarms and calls stay allowed.
 */
final class LiveMode {
    private static final Uri CONDITION = Uri.parse("condition://de.toaster.mic/live");
    private LiveMode() { }

    static void set(Context c, boolean on) {
        NotificationManager nm = c.getSystemService(NotificationManager.class);
        if (!nm.isNotificationPolicyAccessGranted()) return;
        SharedPreferences p = c.getSharedPreferences("live", Context.MODE_PRIVATE);
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                String id = p.getString("rule", null);
                if (id == null || nm.getAutomaticZenRule(id) == null) {
                    if (!on) return;
                    ZenPolicy policy = new ZenPolicy.Builder()
                        .disallowAllSounds()
                        .allowMedia(true)
                        .allowAlarms(true)
                        .allowCalls(ZenPolicy.PEOPLE_TYPE_ANYONE)
                        .allowRepeatCallers(true)
                        .build();
                    AutomaticZenRule rule = new AutomaticZenRule("Toaster Mic Live", null,
                        new ComponentName(c, ToasterActivity.class), CONDITION, policy,
                        NotificationManager.INTERRUPTION_FILTER_PRIORITY, true);
                    id = nm.addAutomaticZenRule(rule);
                    p.edit().putString("rule", id).apply();
                }
                nm.setAutomaticZenRuleState(id, new Condition(CONDITION, "Toaster Mic Live",
                    on ? Condition.STATE_TRUE : Condition.STATE_FALSE));
            } else {
                // Android 9: global filter, restored afterwards.
                if (on && !p.getBoolean("active", false)) {
                    p.edit().putInt("filter", nm.getCurrentInterruptionFilter()).putBoolean("active", true).apply();
                    nm.setNotificationPolicy(new NotificationManager.Policy(
                        NotificationManager.Policy.PRIORITY_CATEGORY_ALARMS | NotificationManager.Policy.PRIORITY_CATEGORY_MEDIA
                            | NotificationManager.Policy.PRIORITY_CATEGORY_CALLS | NotificationManager.Policy.PRIORITY_CATEGORY_REPEAT_CALLERS,
                        NotificationManager.Policy.PRIORITY_SENDERS_ANY, NotificationManager.Policy.PRIORITY_SENDERS_ANY));
                    nm.setInterruptionFilter(NotificationManager.INTERRUPTION_FILTER_PRIORITY);
                } else if (!on && p.getBoolean("active", false)) {
                    nm.setInterruptionFilter(p.getInt("filter", NotificationManager.INTERRUPTION_FILTER_ALL));
                    p.edit().putBoolean("active", false).apply();
                }
            }
        } catch (RuntimeException ignored) { }
    }
}

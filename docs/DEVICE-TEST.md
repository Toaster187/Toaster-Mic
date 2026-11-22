# Abnahme 0.5

## Bereits geprüft

- **Automatisch** (`scripts/test.ps1`, 37 Prüfungen):
  - Protokoll v3: jedes manipulierte Byte, Kürzungen, 10.000 Zufallsdatagramme, MTU-Grenze für 1–4 Kanäle.
  - Handshake, Kopplung, WAKE inkl. Fälschung und Wiedereinspielen.
  - Alle Jitter-Puffer-Fälle aus 0.4.
  - Windows-Empfänger gegen den Handy-Simulator: langsamer Treiber, 20 schnelle Wechsel, Wecken aus dem Ruhezustand.
  - Studio-DSP: Laufzeiten ±1 Sample, +4,35 dB Rauschabstand bei drei ähnlichen Mikrofonen (Theorie +4,33), mit schwachem Mikrofon +2,66 dB (Theorie +2,65), nie über −1 dBFS, −60-dBFS-Rohsignal wird auf etwa −21 dBFS angehoben, lückenlose Schnellstart-Überblendung, keine Allokation, 0,6 ms pro 10-ms-Block.
- **Android-Emulator** (Android 17, x86_64):
  - Oberfläche (Slint), zwei Prozesse (UI und `:audio`), AAudio-Aufnahme, verschlüsselter Empfang am PC.
  - Kopplung, Ruhezustand ohne Aufnahme: 0 CPU-Ticks in 20 s.
  - WAKE → erster Ton nach 80–90 ms, STOP → zurück in den Ruhezustand.
  - Live-Regel aktiv während der Übertragung, danach aus; Richtlinie: Anrufe, Wiederholungsanrufe, Wecker und Medien erlaubt.
  - Neustart des Ruhezustands nach beendetem Prozess.
- **Windows** (ohne Audiogerät, weil die Testumgebung keinen Zugriff auf Windows-Audio hat):
  - Im Infobereich 0 ms CPU in 20 s, 3,5 MB privater Speicher.
  - Empfang 0,5 % eines Kerns.
  - Das Programm benötigt nur Windows-System-DLLs.

## Noch offen – bitte am echten Gerät prüfen

| Test | Durchführung | Erwartung |
| --- | --- | --- |
| VB-CABLE | Windows-App, CABLE Input; OBS/Discord mit CABLE Output | Pegel im Zielprogramm, keine Aussetzer |
| Pixel 9 Mono/Stereo | je 60 s, Stereo nach „Mikrofone zuordnen“ | Stereo: unterschiedliche Kanäle oben/unten |
| Pixel 9 Studio | 3 Rohkanäle, Status „Studio · 3 Mikrofone“ | PC-Status „Verbunden · Studio“, lauter und klarer als Mono (Hörtest) |
| CPU Handy | `adb shell top -p $(adb shell pidof de.toaster.mic:audio)` während Mono | deutlich unter 0.4 (Java) |
| Akku | `adb shell dumpsys batterystats --reset`, 30 min gesperrt übertragen, dann `dumpsys batterystats` | Verbrauch notieren |
| Ruhezustand | Always-On 1–2 h bei ausgeschaltetem Bildschirm, dann `dumpsys batterystats` | Anteil von Toaster Mic praktisch null |
| Wecken | `toaster-probe capture --seconds 20` am PC (öffnet CABLE Output) | `first_audio_ms` notieren (WLAN-Stromsparen, erwartet 0,3–1 s) |
| Übergang | wie oben, Studio-Modus, über 10 s hinaus aufnehmen | `max_gap_ms` ≤ 500, erwartet < 50 |
| Doze | `adb shell dumpsys deviceidle force-idle`, dann Programm öffnet Mikrofon | Handy wacht auf |
| Neustart Handy | Always-On an, Handy neu starten | Benachrichtigung „Toaster Mic bereitstellen“, ein Tipp genügt |
| Live | Testbenachrichtigung `adb shell cmd notification post -S bigtext test "Test"` während Übertragung | kein Ton, keine Vibration; Anruf/Wecker hörbar |
| IP-Wechsel | Handy-WLAN neu verbinden (neue IP), danach Programm öffnet Mikrofon | PC übernimmt die neue IP (ANNOUNCE) |

Je Lauf festhalten: Handy, Android-Version, Akkueinstellung, Router/Band, Modus, Profil, Pufferziel, Aussetzer und gemessene Verzögerung.

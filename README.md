# Toaster Mic 0.5 (Android + Windows, Rust)

WLAN-Mikrofon: das Handy nimmt auf, der PC spielt das Signal in VB-CABLE ab, und jedes Programm nutzt **CABLE Output** als Mikrofon. Beide Apps sind ab 0.5 in Rust geschrieben und auf geringen Akku-, CPU- und Speicherbedarf im Hintergrund ausgelegt. Android 9+, Windows 10/11 x64.

## Download

Jeder Stand von `main` wird automatisch gebaut, getestet und als neuester Release veröffentlicht:

- **Windows-Installer:** [ToasterMic-Setup.exe](https://github.com/Toaster187/Toaster-Mic/releases/latest/download/ToasterMic-Setup.exe) (ohne Administratorrechte, Deinstallation über die Windows-Einstellungen)
- **Android:** [ToasterMic.apk](https://github.com/Toaster187/Toaster-Mic/releases/latest/download/ToasterMic.apk)
- Windows ohne Installer: [ToasterMic-Windows-x64.zip](https://github.com/Toaster187/Toaster-Mic/releases/latest/download/ToasterMic-Windows-x64.zip)

## Starten

- **Windows:** Installer ausführen. VB-CABLE muss installiert sein; **CABLE Input** wird automatisch als Audioziel gewählt.
- **Android:** APK installieren. Neuere Releases installieren sich über ältere. Eine Version 0.4 muss vorher einmal **deinstalliert** werden, weil sie mit einem anderen Schlüssel signiert ist.

1. Handy und PC im selben lokalen Netz.
2. Auf dem Handy **Mikrofon starten**, Mikrofonberechtigung erteilen.
3. Am PC die angezeigte **Handy-IP** eintragen und **Verbinden**.
4. Im Zielprogramm (Discord, OBS, …) **CABLE Output** als Mikrofon wählen.

## Modi

- **Mono:** automatische Android-Eingabe (Rohsignal, wenn das Gerät es anbietet).
- **Stereo:** zwei getrennte Mikrofone. Auf dem Pixel 9 werden die drei Rohkanäle gelesen und die zugeordneten Mikrofone oben und unten übertragen. Davor einmal **Einstellungen → Mikrofone zuordnen** (Ruhe, oben, unten, hinten je drei Sekunden). Auf anderen Geräten genügt die Zuordnung von oben und unten; externe Stereo-Eingänge brauchen keine Zuordnung.
- **Studio:** alle eingebauten Mikrofone (Pixel 9: drei) gehen **unverarbeitet** an den PC. Erst dort werden sie zu einer lauten, klaren Stimme verrechnet, damit der Handy-Akku geschont wird. Die Kette: 80-Hz-Hochpass, Laufzeitausgleich zwischen den Mikrofonen (GCC-PHAT), Gewichtung nach Signal-Rausch-Abstand, Pegelautomatik bis +40 dB, Rauschunterdrückung (RNNoise, *Aus/Mittel/Stark* in den PC-Einstellungen), Kompressor und Limiter bei −1 dBFS. Zusätzliche Verzögerung etwa 22 ms, PC-Last etwa 6 % eines Kerns.

## Live-Schalter (Nicht stören)

Während der Übertragung aktiviert Android die eigene Regel **„Toaster Mic Live“**: Benachrichtigungen, Nachrichten, Erinnerungen, Termine und Systemtöne sowie deren Vibration sind stumm. **Anrufe, Wecker und Medien bleiben erlaubt.** Nach dem Stoppen wird die Regel wieder ausgeschaltet, auch nach einem Absturz beim nächsten Start. Einmalig muss in den App-Einstellungen der Zugriff auf „Nicht stören“ erlaubt werden.

## Always-On

Optional auf beiden Seiten einschaltbar: Handy-Schalter **Always-On** und PC-Einstellung **Always-On** plus **Mit Windows starten**.

- **Kopplung:** einmal manuell verbinden, während Always-On eingeschaltet ist. Der PC übergibt dann ein Kopplungsgeheimnis. Danach reagiert das Handy im Ruhezustand nur noch auf authentifizierte Weckpakete dieses PCs; Wiederholungen alter Pakete werden abgewiesen.
- **Ruhezustand am Handy:** Der kleine Hintergrundprozess `:audio` (ohne Oberfläche) wartet auf dem UDP-Port. Es gibt keine Aufnahme, keinen Wake-Lock, keinen WLAN-Lock und keine Timer; die CPU darf schlafen. Im Emulator wurden in 20 Sekunden 0 CPU-Ticks gemessen.
- **Ruhezustand am PC:** Symbol im Infobereich, kein Audiostream. Windows meldet ereignisgesteuert, wann ein Programm **CABLE Output** öffnet (0 ms CPU in 20 Sekunden gemessen).
- **Wecken:** Sobald ein Programm das Mikrofon öffnet, weckt der PC das Handy. Das Handy sendet sofort, ohne auf eine weitere Bestätigung zu warten. Die ersten 10 Sekunden laufen im **Schnellstart**: niedrigste Latenz, und im Studio-Modus das beste Einzelmikrofon, während die Studio-Kette im Hintergrund einschwingt. Danach wird zeitgleich überblendet; die Aufnahme am Handy läuft dabei durch, es entsteht keine Lücke. 5 Sekunden nachdem das Programm das Mikrofon freigegeben hat, kehrt das Handy in den Ruhezustand zurück.
- Ändert sich die Handy-IP, meldet sich das Handy authentifiziert beim gekoppelten PC (UDP 8126).
- **Android-Grenze:** Seit Android 14/15 darf ein Mikrofondienst nach einem Neustart des Handys nicht von selbst starten. Es erscheint die Benachrichtigung **„Toaster Mic bereitstellen“**; ein Tipp genügt bis zum nächsten Neustart. Das Öffnen der App startet die Bereitschaft ebenfalls. Akkunutzung für Toaster Mic auf „uneingeschränkt“ stellen.

## Latenz und Akku

Einstellung am Handy: **Niedrigste Latenz** (Standard, 5-ms-Pakete, WLAN im Low-Latency-Modus), **Ausgewogen** (10-ms-Bündel) oder **Akku zuerst** (20-ms-Bündel, WLAN darf stromsparen). Gebündelte Pakete werden mit einem einzigen Systemaufruf gesendet. Der PC passt seinen Puffer automatisch an (Vorgaben ab 5 / 15 / 30 / 60 ms, adaptive Reserve, Kürzen in Sprechpausen wie in 0.4).

## Sicherheit

Ephemerer P-256-Schlüsselaustausch, AES-256-GCM für Audio, HMAC-SHA256 für Steuerpakete (Protokoll v3, [docs/PROTOCOL.md](docs/PROTOCOL.md)). Ohne Kopplung kann sich wie bisher jeder PC im lokalen Netz an ein manuell gestartetes Handy anmelden. Nur im vertrauenswürdigen Netz verwenden. Version 0.4 ist nicht kompatibel.

## Bauen und prüfen

Voraussetzungen: Rust (rustup, `stable-x86_64-pc-windows-gnu`, Targets `aarch64-linux-android` und optional `x86_64-linux-android`), Android SDK mit NDK und Build-Tools, JDK (z. B. aus Android Studio). Visual Studio, Gradle und .NET werden nicht benötigt. `scripts/env.ps1` richtet clang/lld aus dem NDK als Linker ein.

```powershell
./scripts/build.ps1                  # Windows-Paket + Android-APK (arm64), Ergebnis in artifacts/
./scripts/build.ps1 -Target Android -Emulator   # zusätzlich x86_64 für den Emulator
./scripts/test.ps1                   # alle automatischen Prüfungen
```

Aufbau: `crates/toaster-core` (Protokoll, Krypto, Jitter-Puffer, Telefon-Logik, Simulator), `crates/toaster-dsp` (Studio-Kette), `crates/toaster-android-audio` (AAudio, JNI, Einstellungen, Statusdatei), `apps/windows`, `apps/android-engine` (`:audio`-Prozess), `apps/android-ui` (Slint-Oberfläche), `apps/android` (Manifest und die von Android vorgeschriebenen Java-Klassen: Dienst, Activity-Hülle, Nicht-stören-Regel, Boot-Hinweis), `tools/toaster-probe` (Handy-Simulator, PC-Empfänger ohne Audiogerät, Aufnahme-Messung).

`toaster-probe`: `phone` simuliert ein Handy (Mono/Stereo/Studio, Always-On), `pc --host IP` empfängt ohne Audiogerät, `capture` öffnet CABLE Output wie ein Aufnahmeprogramm und misst die Zeit bis zum ersten Ton und Lücken.

Hardware-Abnahme: [docs/DEVICE-TEST.md](docs/DEVICE-TEST.md). Die historischen Treiberquellen unter `driver/` und `setup/` sind nicht Teil des Builds.

## Releases

`.github/workflows/release.yml` baut bei jedem Push auf `main` (Windows-Runner, MSVC, NDK r28c), führt alle Tests aus und veröffentlicht `v<Version>.<Build>` mit Installer, APK und ZIP. Die APK wird mit dem Release-Schlüssel aus den Repository-Secrets `ANDROID_KEYSTORE_BASE64` und `ANDROID_KEYSTORE_PASSWORD` signiert (Alias `toastermic`). Lokale Builds verwenden ihn, wenn `TOASTER_KEYSTORE`, `TOASTER_KEYSTORE_PASSWORD` und `TOASTER_KEY_ALIAS` gesetzt sind, sonst den Debug-Schlüssel. Der Installer entsteht lokal mit `./scripts/build.ps1 -Installer`, wenn Inno Setup 6 installiert ist.

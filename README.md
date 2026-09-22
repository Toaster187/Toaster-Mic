# Toaster Mic (Android 0.4.2 / Windows 0.4.1)

Native WLAN-Mikrofon-App für Android 9+ und Windows 10/11 x64. Android verwendet Java/AudioRecord, Windows WPF/WASAPI.

## Starten

- Windows: `artifacts/windows/ToasterMic.exe` oder `ToasterMic-Windows-x64.zip` vollständig entpacken.
- Android: `artifacts/android/ToasterMic-debug.apk` installieren. Bestehende Einstellungen bleiben erhalten.

1. Handy und PC mit demselben erreichbaren lokalen Netz verbinden; Ethernet am PC ist möglich.
2. Auf Android **Mikrofon starten**, Mikrofonberechtigung erteilen.
3. In Windows die angezeigte **Handy-IP** eingeben.
4. Unter **Einstellungen → Audioziel** **CABLE Input (VB-Audio Virtual Cable)** wählen. Ein vorhandenes CABLE Input wird automatisch vorausgewählt.
5. **Verbinden**. In Discord, OBS oder der gewünschten Anwendung **CABLE Output** als Mikrofon wählen.

VB-CABLE muss bereits installiert sein. Die App enthält und installiert keinen eigenen virtuellen Mikrofontreiber oder Windows-Dienst mehr. Treiber, Dienst, Zertifikate und Testmodus-Setup sind aus dem Windows-Paket entfernt. Historische Treiberquellen bleiben im Repository und gehören nicht zum aktuellen Build.

Port 8125 ist die Vorgabe. Falls belegt, auf beiden Geräten denselben anderen Port unter Einstellungen wählen (1024–65535).

## Bedienung

**Android:** Die Hauptseite zeigt Status, IP, Pegel und Start/Stumm. Port, Aufnahmepegel und Akkueinstellungen liegen im Einstellungsmenü. Erst nach Aktivierung von **Stereo** erscheinen Eingabegerät und Links-/Rechts-Zuordnung. Mono verwendet die automatische Android-Eingabe. Port, Stereo und Eingabegerät können vor dem Aufnahmestart geändert werden; Links/Rechts und Pegel auch während der Aufnahme. Mikrofondetails lassen sich separat einblenden.

Auf dem **Pixel 9** erfordert Stereo zunächst **Einstellungen → Mikrofone zuordnen**: Ruhe sowie die Öffnungen oben, unten und hinten jeweils drei Sekunden messen. Gespeichert werden nur Kanalpegel und die eindeutige Zuordnung. Der UNPROCESSED-Aufnahmepfad liest drei Rohkanäle; übertragen werden ausschließlich die gemessenen Mikrofone **unten und oben**, der hintere Kanal wird verworfen. Ohne eindeutige Zuordnung startet internes Stereo nicht. Nach einer Änderung des Android-Builds ist die Messung erneut erforderlich. Die physische Abnahme dieser neuen Zuordnung steht noch aus; Version 0.4.1 nutzte unten und hinten.

Links und rechts verwenden immer unterschiedliche Mikrofone; eine Auswahl tauscht die Zuordnung. Getrennte Pegelanzeigen in den Stereo-Einstellungen helfen beim Prüfen. Echo-Referenz, Telefonie und Loopback stehen nicht mehr als Mikrofone zur Auswahl. Auf anderen Geräten und bei externen Eingängen muss Android zwei unterschiedliche, direkt abgebildete Mikrofone bestätigen. Die Zuordnung steht unter Mikrofondetails.

**Windows:** Kompaktes natives Fenster ohne seitliche Seiten-Scrollleiste. Audioziel, Port und Netzwerkpuffer liegen im separaten Einstellungsfenster und werden vor der Verbindung gewählt. Netzwerkempfang, Geräteabfrage und Audioaufbau/-abbau laufen außerhalb des UI-Threads. Trennen und Schließen warten asynchron auf die Freigabe.

Die Windows-Ausgangslautstärke reicht von **0 bis 1000 %**; 100 % entspricht dem Originalpegel, 1000 % einer Verstärkung um **20 dB**. Android hat weiterhin 0–200 %. Beide Regler multiplizieren sich; zu hohe Werte können hörbar übersteuern. PCM-Werte werden sicher begrenzt. Stumm verbraucht eingehendes Audio weiter, damit beim Einschalten keine alten Samples wiedergegeben werden.

## Puffer und Verbindung

- 48 kHz, 16-Bit-PCM, Mono/Stereo; ein UDP-Paket enthält 240 Frames = 5 ms.
- Puffer-Vorgaben: **ab 5 / 15 / 30 / 60 ms**, Standard ab 15 ms.
- Die App berücksichtigt die Größe der tatsächlichen WASAPI-Abrufe plus ein Reservepaket. Bei 10-ms-Abrufen beginnt auch die kleinste Vorgabe deshalb mit 15 ms Zielreserve.
- Bei Unterläufen steigt die Zielreserve schrittweise bis 60 ms. Ein leerer Puffer wartet auf neue Pakete, statt deren Sequenz dauerhaft vorauszulaufen. Fehlstellen werden kurz ausgeblendet, die Wiederaufnahme eingeblendet.
- Nach mindestens einer Sekunde leiser Audiodaten und zwei Sekunden ohne Unterlauf sinkt eine erhöhte Reserve einmal pro Sekunde um bis zu 10 ms. Dabei werden nur zusammenhängende, tatsächlich leise Pakete vor wartender Sprache verworfen, maximal 40 ms je Schritt. Die Verbindung und der Schlüssel bleiben erhalten. Die gewählte Mindestreserve und die aktuelle Geräteperiode werden weiterhin respektiert.
- Ein einmalig großer WASAPI-Startabruf bestimmt nicht mehr dauerhaft die Mindestreserve. Die Geräteperiode wird in Sekundenfenstern neu bewertet. Lautes Audio, fehlende Pakete und die Stummschalttaste lösen keine Sprechpausen-Kürzung aus. Die Erkennung berücksichtigt die Ausgangsverstärkung und ist bewusst konservativ: bei erhöhtem Hintergrundpegel kann die Kürzung ausbleiben.
- Die Hauptseite zeigt Pufferfüllstand und Ziel; Paketstatistiken und die in Sprechpausen gekürzte Dauer stehen im Tooltip.
- Rückstau wird begrenzt. WASAPI nutzt den geteilten, ereignisgesteuerten Modus mit angefragten 10 ms.
- Die Anzeige misst keine Ende-zu-Ende-Latenz. Android-Aufnahme, Paketankunft, Audiogerät, VB-CABLE und Zielprogramm beeinflussen das Ergebnis ebenfalls.
- Direkte IPv4/UDP-Verbindung mit automatischem ECDH-P-256-Schlüsselaustausch; AES-GCM für Audio, HMAC-SHA256 für Steuerpakete. Das Protokoll bleibt mit Version 0.3.x kompatibel.
- Es gibt keinen Verbindungscode und keine Geräteauthentifizierung. Ein erreichbarer PC kann sich bei freier Verbindung anmelden; nur im vertrauenswürdigen lokalen Netz verwenden.
- Ein PC gleichzeitig; Keepalive und Wiederverbindung bei unveränderter Handy-IP. Gastnetz-Geräteisolierung verhindert die Verbindung.

Die Android-Aufnahme läuft als Foreground-Service mit sichtbarer Benachrichtigung bei gesperrtem Bildschirm weiter. Die Benachrichtigung bietet Stumm/Stoppen. Bei herstellerseitigen Energiesparproblemen die Akkunutzung für Toaster Mic auf uneingeschränkt stellen.

## Bauen und prüfen

Voraussetzungen: .NET 8 SDK, JDK 17, Android SDK 35/Build Tools 35.0.0. Der Windows-Build benötigt kein WDK mehr. Lokale Werkzeuge unter `.tools/` werden bevorzugt.

```powershell
./scripts/build.ps1                 # Windows-Paket + Android-APK und Lint
./scripts/build.ps1 -Target Windows
./scripts/build.ps1 -Target Android
./scripts/test.ps1                  # Protokoll, Audio, Windows-Lebenszyklus und WPF-Layout
```

Automatisiert geprüft: 69 Core-/Protokoll-/Legacy-Brückenprüfungen und 9 Windows-Prüfungen, einschließlich Java/C#-Interop, 10.000 ungültiger Datagramme, UDP-Handshake, Mono/Stereo, Verstärkung, Stumm, adaptive Pufferung, Wiederaufnahme nach Aussetzern und Kürzung in Sprechpausen. Der Windows-Test betreibt den echten Netzwerkempfänger auf einem WPF-Dispatcher, simuliert einen langsamen Audiotreiber und prüft 20 schnelle Verbindungswechsel. WPF-Layoutbilder entstehen unter `artifacts/tests/`.

Zusätzlich auf einem Pixel 9 mit Android 17 geprüft: direkte Zuordnung des unteren/hinteren Mikrofons, Aufnahmedienst, getrennte Links-/Rechts-Auswahl und 600 verschlüsselte Stereo-Pakete über das echte WLAN zum PC. Beide Kanäle enthalten unterschiedliche Signale. Es wurden nur Messwerte gespeichert, keine Tonaufnahme. Die Diagnose-Instrumentation liegt ausschließlich im Debug-Quellsatz und läuft nur bei ausdrücklichem Aufruf.

Noch offen sind räumlicher Hörtest, längerer WLAN-/VB-CABLE-Betrieb und Ende-zu-Ende-Latenzmessung. Die automatisierten Windows-Lebenszyklustests verwenden eine simulierte Audioausgabe. Die Android-APK ist eine mit lokalem Entwicklungsschlüssel signierte Testversion.

Prüfliste: [docs/DEVICE-TEST.md](docs/DEVICE-TEST.md). Drahtformat: [docs/PROTOCOL.md](docs/PROTOCOL.md).

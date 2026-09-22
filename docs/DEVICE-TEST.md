# Hardware-Abnahme

## Android 0.4.2: oben/unten (physische Messung noch offen)

Drei UNPROCESSED-Rohkanäle werden per Kanalindexmaske gelesen. Eine geführte lokale Messung ordnet oben, unten und hinten anhand ihrer Pegel zu; mehrdeutige und doppelte Zuordnungen werden abgelehnt. Nur oben/unten gelangen in die beiden übertragenen Kanäle. Kalibrierungslogik automatisiert geprüft; Android-Build und Lint erfolgreich. Installation auf Pixel 9 erfolgt, geführte Messung zur physischen Abnahme geöffnet. Das belegt noch nicht die gewünschte physische Trennung.

## Historische Messung 0.4.1: unten/hinten

Am 22.09.2026 auf dem angeschlossenen **Pixel 9 mit Android 17** geprüft:

- UNPROCESSED, 48 kHz, Stereo: Android meldet `builtin_mic_1` (bottom) direkt auf Kanal 0 und `builtin_mic_2` (back) direkt auf Kanal 1.
- VOICE_RECOGNITION meldet nur ein aktives Mikrofon. Zwei gleichzeitig geöffnete Mono-Aufnahmen werden auf dasselbe untere Mikrofon geroutet und sind daher keine Lösung für dieses Gerät.
- Der tatsächliche Toaster-Mic-Aufnahmedienst bestätigt die getrennte Zuordnung. Links-/Rechts-Auswahl wurde im laufenden Dienst gewechselt; beide Ausgänge bleiben unterschiedlich zugeordnet.
- Verschlüsselte WLAN-Übertragung an den lokalen PC: **600 Pakete / 144.000 Stereo-Frames**, 136.363 Frame-Paare mit unterschiedlichen Samples. RMS links 23,36, rechts 25,09; Korrelation 0,744. Zusammen mit der direkten physischen Zuordnung bestätigt dies zwei getrennte Audioquellen. Gespeichert wurden nur Messwerte, keine Tonaufnahme.
- Die Puffer-Rückführung ist mit simulierten Audio-/Paketfolgen geprüft: erhöhte Reserve fällt in leisen Pausen zurück; wartende Sprache, kontinuierliche Sprache, Stummschaltung und verstärkte leise Sprache werden nicht als wegwerfbare Stille behandelt.

Messdaten liegen lokal unter `artifacts/pixel-audio-probe.json`, `artifacts/pixel-service-probe-run.txt` und `artifacts/tests/pixel-network-stereo.json`. Noch offen sind räumlicher Hörtest, lang laufender WLAN-/VB-CABLE-Betrieb und gemessene Ende-zu-Ende-Latenz. Die folgende Liste bleibt für die vollständige Abnahme relevant.

| Test | Durchführung | Erwartung |
| --- | --- | --- |
| Android-Hauptseite | App in Mono öffnen | Status, IP, Pegel, Start/Stumm; Einstellungen im Menü |
| Stereo-Menü | Einstellungen öffnen; Stereo an/aus | Eingabegerät und Links/Rechts nur bei Stereo sichtbar |
| Berechtigung | Mikrofonrecht ablehnen, dann erlauben | Klare Meldung; Start nach Erlaubnis |
| Mono | Aufnahme starten und verbinden | Automatische Eingabe; beide Ausgabekanäle gleich |
| Stereo | Zwei verfügbare Kanäle aufnehmen und vertauschen | Tatsächliche Trennung prüfen; Hardwaregrenzen notieren |
| Einstellungen | Port/Gerät ändern, App neu öffnen | Werte gespeichert; Aufnahmeparameter während Aufnahme gesperrt |
| Windows-Layout | Fenster bis Mindestgröße verkleinern; 100/150/200 % Skalierung | Alle Bedienelemente erreichbar, keine Seiten-Scrollleiste |
| VB-CABLE | CABLE Input in Toaster Mic, CABLE Output in OBS/Discord | Audiopegel im Zielprogramm |
| Trennen | Während Audioempfang mehrfach trennen/verbinden | Fenster reagiert weiter; keine alte Audioausgabe |
| Schließen | Beim Aufbau und beim Trennen Fenster schließen | Kontrolliertes Ende ohne Task-Manager |
| Stumm | Mehrere Sekunden stumm, dann Ton an | Sofort aktuelle Aufnahme, keine alten gepufferten Samples |
| Verstärkung | Windows 100/200/500/1000 % bei leisem Eingang | Messbar 0/+6/+14/+20 dB bis zur PCM-Begrenzung |
| Puffer | Kleinste Vorgabe und 15/30/60 ms vergleichen | Ziel berücksichtigt Audioabrufe; nach Aussetzern Erholung |
| Netzverlust | WLAN kurz ausschalten und erneut verbinden | Begrenzter Rückstau, automatische Wiederverbindung |
| Aufnahme-Neustart | Handy-Aufnahme stoppen und erneut starten | Neuer Schlüssel, neuer Pufferzustand |
| Hintergrund | 30–60 Minuten mit gesperrtem Handy | Fortgesetzte Übertragung; Akku-Ausnahmen dokumentieren |
| Gerätestörung | Ausgabe deaktivieren oder USB-Mikrofon abziehen | Verständliche Fehlermeldung, bedienbares Fenster |
| Latenz | Impuls direkt und über Toaster Mic aufnehmen | Differenz messen; Pufferanzeige separat notieren |

Handymodell, Android-Version, Akkueinstellung, Router/Band, Windows-Ausgabe, Pufferziel, Aussetzer und gemessene Ende-zu-Ende-Verzögerung pro Lauf festhalten.

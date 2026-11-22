# Toaster Mic UDP v3

Alle Mehrbyte-Zahlen sind little-endian. Das Handy lauscht standardmäßig auf UDP 8125, ein gekoppelter PC im Always-On-Modus auf UDP 8126. Implementierung: `crates/toaster-core` (`wire.rs`, `keyx.rs`, `phone.rs`); Android und Windows verwenden denselben Code.

## Schlüsselaustausch (`TMK2`, unverändert seit v2)

80 Byte: `TMK2`, Typ (1 = PC, 2 = Handy), drei Nullbytes, 64-Bit-Sitzungs-ID, öffentliche P-256-Koordinaten X und Y (je 32 Byte big-endian). Schlüssel = SHA-256 über die 32-Byte-ECDH-x-Koordinate; derselbe Schlüssel dient für AES-256-GCM und HMAC-SHA256. Das Handy beantwortet identische Angebote identisch, erzeugt höchstens vier neue Schlüsselpaare pro Sekunde und hält einen ausstehenden Austausch 3,5 s. Der PC wiederholt das Angebot jede Sekunde, solange kein Audio eintrifft.

## Datagramm

36 Byte Header, Nutzdaten, 16 Byte Tag. Jedes Datagramm ist höchstens 1472 Byte groß und wird daher nie IP-fragmentiert.

| Offset | Größe | Bedeutung |
| --- | --- | --- |
| 0 | 4 | `TMIC` |
| 4 | 1 | Version `3` |
| 5 | 1 | Typ: HELLO 1, AUDIO 2, STOP 3, PAIR 4, WAKE 5, ANNOUNCE 6 |
| 6 | 1 | Kanäle: Audio 1–4, sonst 0 |
| 7 | 1 | Flags (Audio): Bit 0 stumm, Bit 1 Studio, Bits 2–3 Latenzprofil (0 niedrig, 1 ausgewogen, 2 Akku), Bit 4 reserviert (Wiedergabe-Referenz), Bit 5 Schnellstart |
| 8 | 8 | Sitzungs-ID |
| 16 | 4 | Audio-Sequenz |
| 20 | 4 | `48000` |
| 24 | 2 | Frames: 240 bei 1–2 Kanälen (5 ms), 160 bei 3–4 Kanälen (3,33 ms); Steuerung 0 |
| 26 | 2 | Länge der Steuer-Nutzdaten (≤ 128), Audio 0 |
| 28 | 8 | Zähler (PAIR, WAKE, ANNOUNCE), sonst 0 |

**Audio:** PCM16 verschachtelt, AES-256-GCM, Nonce = Header-Bytes 8–19, der gesamte Header ist AAD. Die Sequenz wird pro Schlüssel nie wiederverwendet; vor einem Überlauf beendet das Handy die Aufnahme.

**Steuerung:** Tag = erste 16 Byte HMAC-SHA256 über Header und Nutzdaten.

- **HELLO** (PC → Handy, jede Sekunde): bestätigt den Schlüsselbesitz und hält die Verbindung 3,5 s. Eine aktive Verbindung wird nicht von einem anderen Endpunkt übernommen.
- **STOP** (PC → Handy): beendet die Übertragung.
- **PAIR** (PC → Handy, Sitzungsschlüssel): Nutzdaten = 32-Byte-Kopplungsgeheimnis XOR HMAC(Sitzungsschlüssel, `TMIC-PAIR` ‖ Sitzung ‖ Zähler). Das Handy speichert Geheimnis, PC-Adresse und Mindestzähler und antwortet mit einem leeren PAIR als Bestätigung.
- **WAKE** (PC → Handy, Kopplungsgeheimnis): Nutzdaten = TMK2-Angebot des PCs mit derselben Sitzungs-ID. Der Zähler muss größer sein als jeder zuvor angenommene. Das Handy antwortet mit TMK2 und sendet sofort Audio. Wiederholungen desselben WAKE (alle 150 ms) erhalten dieselbe Antwort. Im Ruhezustand ignoriert das Handy alles außer gültigem WAKE.
- **ANNOUNCE** (Handy → PC:8126, Kopplungsgeheimnis): Nutzdaten = UDP-Port des Handys; die Absenderadresse ist die neue Handy-IP. Der Zähler muss steigen.

## Grenzen

Der Austausch schützt gegen passives Mitlesen, nicht gegen einen aktiven Angreifer beim ersten, ungekoppelten Verbinden. Steuerpakete ohne Zähler (HELLO/STOP) sind nicht gegen Wiedereinspielen innerhalb einer Sitzung geschützt. Das Protokoll ist für lokale Netze gedacht, nicht für ins Internet weitergeleitete Ports.

# Toaster Mic UDP v2

Alle Mehrbyte-Zahlenfelder sind little-endian. Telefon lauscht standardmäßig auf UDP 8125; der PC nutzt einen vom Betriebssystem vergebenen Quellport. Die Implementierungen liegen in `Wire.java`/`Wire.cs` und `KeyExchange.java`/`KeyExchange.cs`. Version 1 mit manuellem Verbindungscode wird nicht mehr akzeptiert.

## Automatischer Schlüsselaustausch

Der PC erzeugt pro Verbindung ein ephemeres P-256-Schlüsselpaar und eine zufällige 64-Bit-Sitzungs-ID. Sein Angebot hat exakt 80 Byte: `TMK2` (0–3), Typ `1` (4), drei Nullbytes (5–7), Sitzungs-ID (8–15), öffentliche X-Koordinate (16–47), öffentliche Y-Koordinate (48–79). Koordinaten sind jeweils 32 Byte big-endian. Die Antwort hat Typ `2`, dieselbe Sitzungs-ID und den öffentlichen P-256-Schlüssel des Handys.

Beide Seiten berechnen SHA-256 über das 32-Byte-ECDH-Geheimnis als AES-/HMAC-Schlüssel. Ungültige Punkte, Längen, Typen und Sitzungen werden verworfen. Das Handy reserviert einen ausstehenden Austausch für maximal 3,5 Sekunden und erzeugt höchstens vier neue Schlüsselpaare pro Sekunde. Wiederholte identische Angebote erhalten dieselbe Antwort. Erst ein korrektes HMAC-HELLO bestätigt den Schlüsselbesitz und startet die Audioübertragung; Angebote allein verlängern keine aktive Audioverbindung.

Der PC wiederholt das Angebot jede Sekunde, solange kein aktuelles Audio vorliegt. Eine neue Handy-Aufnahme erzeugt einen frischen Schlüssel; der PC akzeptiert einen Schlüsselwechsel erst nach Ablauf seiner Audio-Verbindungsfrist und leert dann den Jitterpuffer. Eine aktive Verbindung wird nicht von einem anderen Endpunkt übernommen.

Es gibt keine Geräteauthentifizierung und keinen Verbindungscode. Der Austausch schützt gegen passives Mitlesen, nicht gegen aktive Man-in-the-Middle-Angriffe. Jeder erreichbare PC kann eine freie Verbindung anfordern. Deshalb für ein vertrauenswürdiges lokales WLAN vorgesehen.

## Audio und Steuerung

Ein Datagramm enthält 36 Byte Header, optional Audiodaten und 16 Byte Authentifizierungstag.

| Offset | Größe | Bedeutung |
| --- | --- | --- |
| 0 | 4 | ASCII `TMIC` |
| 4 | 1 | Version `2` |
| 5 | 1 | Typ: HELLO `1`, AUDIO `2`, STOP `3` |
| 6 | 1 | Kanäle: Audio `1` oder `2`, Steuerung `0` |
| 7 | 1 | Flags: Bit 0 = Handy stumm |
| 8 | 8 | Zufällige PC-Sitzungs-ID |
| 16 | 4 | Audio-Sequenznummer, unsigned interpretiert |
| 20 | 4 | Sample-Rate `48000` |
| 24 | 2 | Frames: Audio `240`, Steuerung `0` |
| 26 | 2 | Reserviert, `0` |
| 28 | 8 | Reserviert für Zeitstempel, derzeit `0` |
| 36 | variabel | AES-GCM-verschlüsseltes PCM16, L/R verschachtelt |
| Ende − 16 | 16 | GCM-Tag für Audio; gekürztes HMAC für Steuerung |

Audio: AES-256-GCM, 128-Bit-Tag. Die 12-Byte-Nonce besteht aus den Headerbytes 8–19 (Sitzung + Sequenz); der vollständige 36-Byte-Header ist AAD. Die Sequenz läuft während der gesamten Android-Aufnahme weiter, auch bei Clientwechseln. Vor einem Sequenzumlauf stoppt der Dienst. Dadurch wird innerhalb einer Aufnahme kein Schlüssel-/Nonce-Paar erneut verwendet. Nach einem Aufnahmestart muss ein neuer Schlüsselaustausch stattfinden.

Steuerung: erste 16 Byte von HMAC-SHA256 über den Header. HELLO und STOP haben weder PCM-Daten noch Verschlüsselungsnonce. Der Windows-Empfänger akzeptiert nur mit dem ausgehandelten Schlüssel validiertes Audio von seinem verbundenen UDP-Endpunkt und seiner aktuellen Sitzung. Private Austauschschlüssel werden nicht dauerhaft gespeichert.

HELLO wird jede Sekunde wiederholt. Android akzeptiert während einer aktiven Verbindung nur denselben Endpunkt und dieselbe Sitzung. Nach 3,5 Sekunden ohne HELLO darf eine neue Sitzung übernehmen. STOP wird nur vom aktiven Peer akzeptiert. Es gibt keine TCP-Warteschlangen oder Audio-Neuübertragung.

Die Implementierung begrenzt Audiopuffer und verwirft verspätete/duplizierte Pakete innerhalb der laufenden Puffersequenz. Dies ist kein allgemein geprüftes Sicherheitsprotokoll; insbesondere sind Steuerpakete nicht durch eine gegenseitige Challenge vor Wiedereinspielen geschützt. Es ist für direkte lokale Verbindungen ausgelegt, nicht für offen ins Internet weitergeleitete Ports.

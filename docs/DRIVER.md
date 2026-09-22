# Virtuelles Windows-Mikrofon

Die Windows-Seite besteht aus drei Teilen:

1. `ToasterMic.sys` ist ein WaveRT-Capture-Treiber auf Basis von Microsofts Simple-Audio-Sample. Er stellt ein 48-kHz-/16-Bit-/Stereo-Aufnahmegerät bereit und liest die PCM-Daten aus einem begrenzten Kernel-Ringpuffer.
2. `ToasterMic.Service.exe` läuft als LocalSystem-Dienst. Die Desktop-App schreibt nur über die lokale Named Pipe `ToasterMic.Audio.v1`; Netzwerkzugriffe auf diese Pipe werden verweigert.
3. Der Dienst aktiviert den PnP-Knoten erst nach gültigem `TMV1`-Handshake und dem ersten vollständigen 5-ms-Audioblock. Bei EOF, Timeout, Trennung, Dienststopp oder Fehler wird der Knoten deaktiviert. Die Deaktivierung wird vor jedem neuen Client und beim Dienststart wiederholt.

## Produktionsinstallation

Ein normal gestartetes aktuelles 64-Bit-Windows benötigt für neue Kernel-Treiber eine Microsoft-Hardware-Signatur. Die automatische lokale Entwicklungssignatur ersetzt diese nicht. Für eine Microsoft-Einreichung werden ein Hardware-Dev-Center-Konto und ein damit verknüpftes EV-Zertifikat benötigt: [Microsoft-Anforderungen](https://learn.microsoft.com/en-us/windows-hardware/drivers/dashboard/code-signing-reqs).

## Automatische Entwicklungssignatur

`scripts/build-driver.ps1` ruft nach jedem erfolgreichen Build automatisch `scripts/sign-driver.ps1` auf. Der Ablauf erzeugt ein kurzlebiges privates RSA-Schlüsselpaar mit Code-Signing-Zertifikat, signiert zuerst die SYS-Datei mit SHA-256, erzeugt dann den Katalog neu und signiert ihn ebenfalls. WinVerifyTrust prüft die Signaturen und Dateihashes. Der erwartete Fehler für einen nicht vertrauten Stamm wird von einer beschädigten Signatur unterschieden.

`artifacts/driver-testsigned` enthält SYS, INF, CAT, das öffentliche Zertifikat und `signing.json` mit Prüfsummen. `driver-unsigned` bleibt als Rohbuild erhalten. Der temporäre, passwortgeschützte PFX-Schlüssel wird auch im Fehlerfall gelöscht; er wird nicht ausgeliefert. Bei jedem Build entsteht ein neues Entwicklungszertifikat. Kein Zertifikat wird in Windows als vertrauenswürdig installiert, Secure Boot und Bootkonfiguration bleiben unverändert.

Die Entwicklungssignatur eignet sich für Windows im Testmodus. Windows-Testsigning und gegebenenfalls Secure-Boot-Einschränkungen beschreibt [Microsoft](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/the-testsigning-boot-configuration-option).

## Einrichtung aus der Windows-App

Ab Windows-App 0.3.1 startet **Virtuelles Mikrofon installieren (Administrator)** das Setup mit `runas` und Windows-UAC. Auch beim ersten Verbinden mit dem integrierten Ziel wird die Einrichtung gestartet, falls der Dienst fehlt. Die Audio-App selbst benötigt dauerhaft keine Administratorrechte. Der PowerShell-Aufruf setzt die Ausführungsrichtlinie nur für diesen Setup-Prozess; die systemweite Richtlinie wird nicht geändert.

Das Setup wählt ein vorhandenes Microsoft-Paket in `driver` bevorzugt aus. Ein beschädigtes Produktionspaket wird nicht durch einen stillen Rückfall auf Testsignierung ersetzt. Fehlt der Ordner, werden SYS-/CAT-Signaturen, Zertifikat-Fingerabdruck und INF-/SYS-/CAT-Prüfsummen des Pakets in `driver-testsigned` geprüft.

Bei Testtreibern wird der tatsächlich aktive Kernel-Code-Integrity-Modus gelesen. Ist Testsigning aus, fragt das interaktive Setup ausdrücklich, ob der Windows-Testmodus vorbereitet werden soll. Nur bei **Ja** wird `bcdedit /set {current} testsigning on` aufgerufen. Das betrifft systemweit die Zulassung von Testtreibern. Secure Boot wird nicht abgeschaltet; blockiert Windows die Änderung, erscheint die Originalfehlermeldung. Nach erfolgreicher Vorbereitung ist ein selbst ausgeführter Neustart und ein erneuter Aufruf der Einrichtung erforderlich (Rückgabecode 3010). Bis dahin werden keine Zertifikate oder Treiber installiert.

Bei aktivem Testmodus importiert Setup das geprüfte öffentliche Zertifikat in die lokalen Computer-Zertifikatspeicher `Root` und `TrustedPublisher`, installiert das Root-Gerät und prüft, ob sich der Treiber-Audiokanal tatsächlich öffnen lässt. Danach wird das Gerät deaktiviert und der Dienst eingerichtet. Die Abschlussprüfung kontrolliert genau ein deaktiviertes Gerät und die Erreichbarkeit des Dienstes. Neu importierte Zertifikate werden bei einem Installationsfehler zurückgenommen; ein angelegtes Gerät wird nach Möglichkeit deaktiviert. Fehler bei der Bereinigung werden angezeigt.

`-ValidateOnly -NoDialog` prüft ausschließlich und verändert weder Bootkonfiguration noch Zertifikate, Dienste oder Geräte. Fehlende Testmodus-Voraussetzungen liefern Exitcode 2, sonstige Fehler 1, eine erfolgreiche Einrichtung 0. UAC-Abbruch wird von der App getrennt behandelt. Der vollständige Ablauf mit geladenem Testtreiber muss auf einem dafür eingerichteten Windows-Testsystem noch praktisch abgenommen werden.

## Einrichtung eines Microsoft-signierten Pakets

Für eine verteilbare Version müssen die drei Dateien `ToasterMic.inf`, `ToasterMic.sys` und `ToasterMic.cat` in einen Ordner `driver` des Windows-Pakets gelegt werden. Der Katalog muss von Microsoft als `Microsoft Windows Hardware Compatibility Publisher` signiert sein. Danach als Administrator ausführen:

```powershell
./setup/Install-VirtualMicrophone.ps1 -NoDialog
```

Das Skript prüft Signatur, Paket und Windows 10 Version 2004 oder neuer (x64), bevor es den Dienst oder den Geräteknoten anlegt. `setup/Disable-VirtualMicrophone.ps1` beendet den Dienst und deaktiviert den Geräteknoten wieder.

## Entwicklungsprüfung

Der Treiber lässt sich mit dem lokalen WDK bauen und der Ringpuffer kann ohne Installation getestet werden:

```powershell
./scripts/build-driver.ps1
./scripts/test.ps1
./scripts/test-signing.ps1
./scripts/test-setup.ps1
```

Die Windows-App zeigt das integrierte Ziel auch ohne Installation an, startet aber keinen Feed und lässt Windows unverändert, solange der signierte Treiber/Dienst nicht vorhanden ist.

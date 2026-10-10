# VCMI-FV Launcher – Hilfe

> Diese Dokumentation wird mit dem Launcher ausgeliefert und aktualisiert sich mit ihm; beim Wechsel der Oberflächensprache wird die passende Fassung angezeigt.

## Schnellstart

### Erster Start

- Beim ersten Start öffnet sich der Assistent: **Spielverzeichnis** (muss `VCMI_client.exe` enthalten) und **Benutzerdatenverzeichnis** (meist `Dokumente\My Games\vcmi`) wählen und speichern.
- Verzeichnisse werden zuerst aus `config/dirs.json` gelesen, sonst gelten die VCMI-Standardregeln.

### Vier Schritte zum Spiel

1. Oben „Welten“: Welt erstellen oder betreten (optional; ohne Welt gilt die globale Konfiguration).
2. Unter „Mods“ links Städte/Erweiterungen wählen und rechts prüfen (Abhängigkeiten werden automatisch gesetzt).
3. Im Preset-Balken „Speichern“ klicken, um `modSettings.json` zu schreiben.
4. Unten „Spiel starten“ klicken.

### Unterer Balken

- Links: Dateien importieren / Spieldaten / Hilfe / offizieller Launcher.
- Rechts: Karteneditor / Spiel starten.

## Mods und Presets

### Was ist ein Preset

- Ein Preset ist eine Sammlung von Mod-Schaltern (`mods`-Array plus Submods `settings[Eltern][Kind]`) in VCMIs `modSettings.json`.
- Der Preset-Balken bietet: Wechseln, Speichern (aktiv bei Änderungen), Neu (mit Vererbung), Umbenennen, Import/Export des offiziellen Formats, Löschen und einen Markdown-Bericht.

### Abhängigkeiten und Konflikte

- Beim Auswählen werden `depends` automatisch gesetzt; `conflicts` werden rechts rot markiert.
- Fehlende Mods, unerfüllte Abhängigkeiten und Konflikte blockieren den Start und werden in einem Dialog gelistet.

### Submods

- Eltern-Mod aufklappen und Unterelemente wählen; sie landen in `settings[Eltern][Kind]`; Leerzeichen und Punkte in IDs bleiben erhalten.

### Startfähige Basis

- Beim Speichern wird die minimale Basis ergänzt (Standard der Enhanced-Version: `vcmi` / `h3e-main-menu` / `core`), damit das Spiel startet.

## Welten

### Wie die Isolation funktioniert

- Eine Welt ist eine eigenständige VCMI-Umgebung: Der Launcher schreibt `config/dirs.json` um (nur die 5 Schlüssel `userDataPath` / `userConfigPath` / `userCachePath` / `userLogsPath` / `userSavePath`) und lenkt die Benutzerordner in die Welt.
- Beim Deaktivieren wird die **Original-Sicherung** wiederhergestellt (nicht „Standardpfade“).
- Weltdaten liegen standardmäßig unter `%LOCALAPPDATA%\VCMI-FV-Launcher\realms\<id>\`.

### Drei Mod-Referenzarten

- **Standard**: Mods aus Benutzer- und Spielverzeichnis werden direkt genutzt (keine Kopie).
- **Integriert**: Mods werden in die Welt kopiert (reisen mit ihr).
- **Geliehen**: Verknüpfung (Junction) auf externe Mods; Aufheben löscht nur die Verknüpfung.

### Kartenventil

- „Spieleigene Karten blockieren“ benennt den `Maps`-Ordner in `Maps.vcmi-fv-native` um und legt eine Junction an (standardmäßig transparent); Ausschalten entfernt die Junction. Jederzeit rückgängig zu machen.

### Betreten und Verlassen

- Betreten aktiviert (dirs.json, Welt-Preset, geliehene Mods); Verlassen deaktiviert und stellt wieder her.
- Die Welt-Oberfläche hat keinen unteren Balken: Spielstart und Editor liegen auf dem Welt-Startbildschirm.

## Einstellungen

### Öffnen

- Die Zahnrad-Schaltfläche oben rechts schaltet die Einstellungen um; erneut klicken (oder einen Tab anklicken) zum Verlassen.

### Anzeigeebene

- Drei Stufen: **Üblich / Erweitert / Alle** (Standard: Üblich).
- Die Suche findet Einträge über alle Stufen hinweg.

### Geltungsbereich

- Die Kennzeichnung „Global gemeinsam genutzt / Nur diese Welt“ oben rechts ist ein **reiner Status**: Hover oder Klick öffnet ein Panel mit Erklärung, in welches Konfigurationsverzeichnis geschrieben wird, und einer Kopier-Schaltfläche.
- Die Launcher-Einstellungen sind immer „Global“; die Welt-Einstellungen folgen der Welt-Richtlinie.

### Menü „Mehr“

- **Laufzeitdaten**: letzte Karte/Spielstand, Sprache der Spieldaten, aktivierte Mods, VCMI-Version – aus dem aktuellen Konfigurationsverzeichnis.
- **Nach Updates suchen**: nur in den Launcher-Einstellungen.
- **Konfigurationsordner öffnen**.
- **Auf Standard zurücksetzen** (rot): setzt alle vom Launcher verwalteten VCMI-Einstellungen dieses Ordners auf die integrierten Werte zurück.

### Schreibsemantik

- Nur vom Standard abweichende Werte landen in `settings.json`; Zurücksetzen löscht den Schlüssel und überlässt ihn VCMI.

## Updates

### Wie Sie von einer neuen Version erfahren

- Roter Punkt am Zahnrad – auf jeder Seite sichtbar.
- Punkt an „Mehr“ und Versionsetikett im Menü – in den Einstellungen.
- Automatischer Dialog: dieselbe Version erinnert **höchstens einmal pro 7 Tage**; eine **neue** Version sofort.
- „Diese Version überspringen“ ignoriert sie dauerhaft; „Nach Updates suchen“ zeigt das Ergebnis weiterhin an.

### Erzwungene Updates

- Verlangt das Manifest eine Mindestversion, blockiert der Launcher mit einem Vollbild-Overlay.

### Download und Installation

- Der Dialog lädt herunter und prüft sha256; reine Konfigurationspakete werden direkt angewendet (kein Neustart), Pakete mit exe erzeugen ein Ersetzungsskript und starten den Launcher neu.

## HD-Materialien

### Quelle

- Wird der offizielle HD-Mod (`hd-edition`) erkannt, entstehen `@2x`/`@3x`-Varianten aus echtem HD; Stadtbildschirme und Kreaturhintergründe nutzen HD-Basisbilder.
- Einstellungen → „FV-Launcher“ (Stufe „Alle“): Quelle (Auto / Aus) und Stufe (Auto / x2).

### KI-Upscaling

- Mit „KI-HD-Verbesserung“ nutzen Illustrationen (Kreaturen / Helden / Gebäude) KI-Upscaling; Pixelart bleibt bei xBRZ.
- Erfordert die Komponente „KI-Laufzeit“ und ein **explizit gewähltes Modell** (ohne Modell kein KI-Einsatz).

### Wirksamkeit

- Nach Änderung von Schalter oder Stufe muss der **Material-Cache neu erstellt** werden.

## Netzwerk

### GitHub-Proxy

- Ist GitHub nicht erreichbar, HTTP(S)-Proxy unter Einstellungen → „FV-Launcher“ → „GitHub-Proxy“ eintragen (z. B. `http://127.0.0.1:7890`); leer = direkt.
- Gilt nur für GitHub-Adressen: erst direkt, bei Verbindungsfehler Proxy. SOCKS5 wird nicht unterstützt.

### Spiegelregeln

- Die Karte „GitHub-Spiegelregeln“ verwaltet eine Spiegelliste: nach direktem Fehler der Reihe nach, dann Proxy.

## Verzeichnisse und Daten

### dirs.json

- VCMI kennt nur 5 Schlüssel (`userDataPath` / `userConfigPath` / `userCachePath` / `userLogsPath` / `userSavePath`); andere Felder werden ignoriert.
- Pfade expandieren Umgebungsvariablen; relative Pfade gelten ab dem Arbeitsverzeichnis des Prozesses – absolute Pfade sind Pflicht.
- `Mods` im Spiel- und Benutzerverzeichnis werden **gleichzeitig** eingebunden; dasselbe gilt für Karten.

### Spieldaten-Panel

- Unten „Spieldaten“: Engine-Version, Core-Mod-Version (nicht die Engine-Version), Betriebssystem, Datenverzeichnisse (im Explorer öffnen), Integritätsprüfung (LOD-Einträge / Core-Mod / Ressourcencontainer).

### Dateien importieren

- „Dateien importieren“ oder Dateien ins Fenster ziehen: `zip` / `h3m` / `h3c` / `vmap` usw.; Zip wird als Mod-Paket erkannt.

## FAQ

### Warnung „Mod fehlt“

- Das Preset verweist auf einen nicht vorhandenen Mod. Auf der Mods-Seite ergänzen oder aus dem Repository installieren; bei Sprachmods erscheint „Aus Repository installieren“.

### Wie wird die VCMI-Version ermittelt?

- Die exe hat keine PE-Version: Der Launcher prüft `VCMI_client.exe -v`, dann PE, dann Scan. Die Core-Mod-Version (z. B. `1.5`) ist die Asset-Version des Mods, nicht die Engine-Version.

### Portable Installation

- Die exe in ein beliebiges Verzeichnis legen; im Assistenten die Ordner angeben; `launcher.json` kann Sprache, Ordner und Schalter vorgeben.

### Warum erscheint die Update-Erinnerung manchmal nicht?

- Optionale Updates sind gedrosselt: einmal pro Version, dann 7 Tage Ruhe (Punkt und Etikett bleiben). Manuell: „Mehr → Nach Updates suchen“.

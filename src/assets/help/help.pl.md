# Pomoc VCMI-FV Launcher

> Dokumentacja jest dołączana do launchera i aktualizowana razem z nim; po zmianie języka interfejsu wyświetlana jest odpowiednia wersja.

## Szybki start

### Pierwsze uruchomienie

- Przy pierwszym uruchomieniu otwiera się kreator: wybierz **katalog gry** (musi zawierać `VCMI_client.exe`) oraz **katalog danych użytkownika** (zwykle `Dokumenty\My Games\vcmi`) i zapisz.
- Katalogi są wykrywane najpierw z `config/dirs.json`, a w razie braku — według domyślnych zasad VCMI.

### Cztery kroki do gry

1. U góry „Światy”: utwórz lub wejdź do świata (opcjonalnie; bez świata używana jest konfiguracja globalna).
2. Na karcie „Mody” wybierz miasta/dodatki po lewej i sprawdź po prawej (zależności zaznaczają się automatycznie).
3. Kliknij „Zapisz” na pasku zestawów, aby zapisać `modSettings.json`.
4. Kliknij „Uruchom grę” na dolnym pasku.

### Dolny pasek

- Po lewej: import plików / dane gry / pomoc / oficjalny launcher.
- Po prawej: edytor map / uruchom grę.

## Mody i zestawy

### Czym jest zestaw

- Zestaw to zbiór przełączników modów (tablica `mods` i podmody `settings[rodzic][dziecko]`) zapisany w `modSettings.json` VCMI.
- Pasek zestawów: przełączanie, zapis (aktywuje się przy zmianach), nowy (z dziedziczeniem), zmiana nazwy, import/eksport oficjalnego formatu, usuwanie, raport Markdown.

### Zależności i konflikty

- Zaznaczenie modu automatycznie zaznacza jego `depends`; konflikty (`conflicts`) są podświetlane na czerwono.
- Brakujące mody, niespełnione zależności i konflikty blokują uruchomienie i są wymieniane w oknie.

### Podmody

- Rozwiń mod nadrzędny i zaznacz podelementy — trafiają do `settings[rodzic][dziecko]`; spacje i kropki w ID są zachowywane.

### Linia bazowa

- Przy zapisie dodawana jest minimalna linia bazowa (domyślnie wersja rozszerzona: `vcmi` / `h3e-main-menu` / `core`), aby gra się uruchamiała.

## Światy

### Jak działa izolacja

- Świat to niezależne środowisko VCMI: launcher nadpisuje `config/dirs.json` (tylko 5 kluczy: `userDataPath` / `userConfigPath` / `userCachePath` / `userLogsPath` / `userSavePath`), kierując katalogi użytkownika do świata.
- Dezaktywacja przywraca **oryginalną kopię zapasową** (nie „domyślne ścieżki”).
- Dane świata domyślnie: `%LOCALAPPDATA%\VCMI-FV-Launcher\realms\<id>\`.

### Trzy sposoby używania modów

- **Domyślnie**: mody z katalogu użytkownika i gry bez kopiowania.
- **Integracja**: kopiowanie modów do katalogu świata (przenoszone razem z nim).
- **Pożyczanie**: dowiązanie katalogowe (junction); usunięcie odwołania nie usuwa plików źródłowych.

### Zawór map

- „Blokuj mapy gry” zmienia nazwę katalogu `Maps` na `Maps.vcmi-fv-native` i tworzy junction (domyślnie w pełni przezroczysty); wyłączenie usuwa junction. Można przywrócić w każdej chwili.

### Wejście i wyjście

- Wejście aktywuje świat (dirs.json, zestaw świata, pożyczone mody); wyjście dezaktywuje i przywraca.
- Interfejs świata nie ma dolnego paska: uruchamianie gry i edytor są na ekranie głównym świata.

## Ustawienia

### Jak otworzyć

- Przycisk koła zębatego u góry po prawej przełącza ustawienia; kliknij ponownie (lub dowolną kartę), aby wyjść.

### Poziom widoczności

- Trzy poziomy: **Zwykłe / Zaawansowane / Wszystkie** (domyślnie „Zwykłe”).
- Wyszukiwarka znajduje pozycje na wszystkich poziomach.

### Zakres

- Etykieta „Globalne współdzielone / Tylko świat” u góry po prawej to **tylko stan**: najechanie lub kliknięcie rozwija panel z wyjaśnieniem, do którego katalogu konfiguracji trafiają ustawienia, i przyciskiem kopiowania ścieżki.
- Strona ustawień launchera jest zawsze „Globalne współdzielone”; strona świata zależy od jego polityki.

### Menu „Więcej”

- **Informacje o środowisku**: ostatnia mapa/zapis, język danych gry, liczba włączonych modów, wersja VCMI — z bieżącego katalogu konfiguracji.
- **Sprawdź aktualizacje**: tylko na stronie ustawień launchera.
- **Otwórz katalog konfiguracji**.
- **Przywróć ustawienia domyślne** (czerwone): przywraca wszystkie zarządzane przez launcher ustawienia VCMI w tym katalogu.

### Semantyka zapisu

- Do `settings.json` trafiają tylko wartości inne niż domyślne; powrót do domyślnej usuwa klucz.

## Aktualizacje

### Jak dowiesz się o nowej wersji

- Czerwona kropka na kole zębatym — widoczna na każdej stronie.
- Kropka na przycisku „Więcej” i etykieta wersji w menu — w ustawieniach.
- Automatyczne okno: ta sama wersja przypomina **nie częściej niż raz na 7 dni**; **nowa** wersja — natychmiast.
- „Pomiń tę wersję” — trwale ignoruje wersję; „Sprawdź aktualizacje” nadal pokazuje wynik.

### Wymuszona aktualizacja

- Gdy manifest określa minimalną obsługiwaną wersję, launcher blokuje działanie pełnoekranowym nakładkiem.

### Pobieranie i instalacja

- Okno pobiera i weryfikuje sha256; pakiety tylko z konfiguracją stosują się na miejscu (bez restartu), pakiety z exe tworzą skrypt podmiany i restartują launcher.

## Materiały HD

### Źródło

- Po wykryciu oficjalnego moda HD (`hd-edition`) warianty `@2x`/`@3x` pochodzą z prawdziwego HD, a tła ekranów miast i stworzeń używają obrazów bazowych w HD.
- Ustawienia → „FV launcher” (poziom „Wszystkie”): źródło (auto / wył.) i skala (auto / x2).

### Skalowanie AI

- Po włączeniu „AI HD enhance” ilustracje (stworzenia / bohaterowie / budynki) używają AI; pixel art nadal xBRZ.
- Wymagany komponent „AI runtime” i **wybrany model** (bez modelu AI nie działa).

### Kiedy zmiany działają

- Po zmianie przełącznika lub skali trzeba **przebudować pamięć podręczną materiałów**.

## Sieć

### Proxy GitHub

- Jeśli GitHub jest niedostępny, ustaw proxy HTTP(S) w ustawieniach → „FV launcher” → „Proxy GitHub” (np. `http://127.0.0.1:7890`); puste = bezpośrednio.
- Działa tylko dla adresów GitHub: najpierw bezpośrednio, proxy przy błędzie połączenia. SOCKS5 nie jest obsługiwany.

### Reguły luster

- Karta „Reguły luster GitHub”: po nieudanym połączeniu bezpośrednim lustra są próbowane po kolei, potem proxy.

## Katalogi i dane

### Mechanizm dirs.json

- VCMI rozumie tylko 5 kluczy (`userDataPath` / `userConfigPath` / `userCachePath` / `userLogsPath` / `userSavePath`); pozostałe są ignorowane.
- Ścieżki rozwijają zmienne środowiskowe; względne są liczone od katalogu roboczego procesu — wymagane są bezwzględne.
- Katalogi `Mods` gry i użytkownika są montowane **jednocześnie**; tak samo mapy.

### Panel „Dane gry”

- Wersja silnika, wersja modu głównego (to nie wersja silnika), system, katalogi danych (otwórz w Eksploratorze), kontrola integralności (wpisy LOD / mod główny / kontenery zasobów).

### Import plików

- Przycisk „Import plików” lub przeciągnięcie do okna: `zip` / `h3m` / `h3c` / `vmap` itd.; zip jest rozpoznawany jako pakiet modów.

## FAQ

### Ostrzeżenie o brakującym modzie

- Zestaw odwołuje się do moda, którego nie ma na dysku. Dodaj go na karcie „Mody” lub zainstaluj z repozytorium; dla modów językowych pojawia się przycisk „Zainstaluj z repozytorium”.

### Skąd pochodzi wersja VCMI

- Plik exe nie ma wersji PE: launcher sprawdza `VCMI_client.exe -v`, potem PE, potem skanowanie. Wersja modu głównego (np. `1.5`) to wersja zasobów modu, nie silnika.

### Wdrożenie przenośne

- Umieść exe launchera w dowolnym katalogu; wskaż katalogi w kreatorze; `launcher.json` może wstępnie ustawić język, katalogi i przełączniki.

### Dlaczego czasem nie ma przypomnienia o aktualizacji

- Aktualizacje opcjonalne są ograniczane: jedna wersja przypomina raz, potem 7 dni ciszy (kropka i etykieta pozostają). Sprawdź ręcznie: „Więcej → Sprawdź aktualizacje”.

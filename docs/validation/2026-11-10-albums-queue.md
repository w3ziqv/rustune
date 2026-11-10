# Albumy i kolejka — walidacja lokalna, 10 listopada 2026

Zakres: pełny kontekst albumu, sesja kolejki, migracja, IPC, TUI/CLI, MPRIS i bezpośrednia integracja natywnego workera. Instrukcje: `docs/queue.md`; plan i tabela akcji: `docs/superpowers/plans/2026-11-10-albums-queue.md`. Zachowano nieśledzony katalog przeglądu i pracę istniejącą przed rozpoczęciem. Brak publikacji, tagów, zmiany instalacji oraz odczytu prywatnych tokenów/CDM użytkownika.

Rust 1.99.1 (`58cf8f9c3`), edycja 2024, Linux. Poniżej wynik końcowej weryfikacji po poprawkach.

| Kontrola | Wynik |
|---|---|
| `cargo fmt --all -- --check` | przeszło |
| `cargo clippy --all-targets -- -D warnings` | przeszło |
| `cargo test` | 145 testów przeszło, 0 niepowodzeń; 3 pominięte w tym przebiegu (dwa sieciowe uruchomione osobno) |
| `cargo build --release` | przeszło; standardowy natywny workspace, systemowy FFmpeg |
| `git diff --check` | przeszło |
| `cargo test --test artwork -- --ignored` | osobno przeszło; publiczna okładka Apple |
| `cargo test --test lyrics -- --ignored` | osobno przeszło; publiczny LRCLIB |
| `scripts/verify-albums-queue.py target/release/rustune` | osobno przeszło w testowym venv z pyte; syntetyczne dane, prywatne XDG i D-Bus |

## Scenariusze przed i po

| Przed | Potwierdzone po zmianie |
|---|---|
| Play dopisuje utwór na końcu starej kolejki | Jawne PlayContext zastępuje sesję; Play utworu w albumie zachowuje cały album i wskazany start |
| Widoczna strona albumu to cały dostępny kontekst | HTTP fixture 225 utworów, dziewięć stron po 25, dwie płyty; start ze środka 225-utworowej sesji przechodzi do jej końca |
| Brak jednej akcji całego albumu | Test TUI → wyszukanie → Enter → pierwsze 25/45 → a → jedna akcja Play → rzeczywiste IPC/usługa → 45 kolejnych sterowanych EOF |
| Play next X, Y daje Y, X | FIFO X, Y, pozostały album; spójne bloki i pierwszeństwo także przed historią do przodu przy shuffle |
| Song ID utożsamia kilka wystąpień | Oddzielne trwałe entry ID, duplikaty zachowane po migracji/restartach; nieaktualny klient nie usuwa innego wystąpienia |
| Usunięcie current odłącza worker od kolejki | Jawne remove-and-advance przejmuje pauzę/playing, a Stopped nie uruchamia następnego; brak następnego kończy audio |
| Shuffle losuje niezależnie, Previous używa indeksu | Stabilna permutacja, historia w shuffle, fizyczny poprzednik przy shuffle off, próg >3 s i odtwarzanie historii do przodu |
| Clear usuwa preferencje i może pozostawić audio bez wpisu | ClearUpcoming zostawia bieżący wpis i stan audio, kończy po EOF; jeden ograniczony Undo przywraca kolejkę bez autoplay |
| Zapis >16 MiB jest akceptowany, lecz nie daje się odczytać | Dokładnie 16 777 216 B daje się zapisać i odczytać; dodatkowy bajt nie zmienia pliku ani kolejki |
| Późna odpowiedź albo event może zmienić nową sesję | Rewizje i generacje odrzucają stare pobrania, także anulowanie → retry z tym samym kluczem, oraz stare eventy audio |
| Błędy audio giną pod statusem ekranu | Trwały Failed ma widoczną treść i Play retry; Stop/Undo usuwa nieaktualny błąd |

Dodatkowe testy: nieudana strona, deklarowana niekompletność, brak numerów (kolejność źródła), niedostępne/nieobsługiwane rekordy z licznikami przyczyn, limity, błąd zapisu, uszkodzony stan/recovery, idempotencja po utracie odpowiedzi i restarcie, brak autoplay przy dodawaniu, repeat off/one/all, edycje w trakcie, EOF i Stop podczas zapisu, błąd fsync po rename, rozdzielenie potwierdzenia kolejki od audio. Test limit-respecting iTunes potwierdza pobranie pełnej ograniczonej relacji zamiast żądania jednej widocznej strony.

Sterowany backend potwierdza Ready/State/Ended/Error przez granicę drivera. Test całego albumu uruchamia też prywatny `dbus-daemon`, sprawdza MPRIS PlaybackStatus oraz obserwuje sygnały PlaybackStatus, Metadata, CanGoNext, CanPause i CanSeek. Test wolnego katalogu potwierdza wysłanie transportu przed zakończeniem metadanych. Testy usługi potwierdzają Pause/Volume/Status w czasie nieukończonego pobierania i zablokowanego zapisu (prywatny FIFO), bez wymagania konta/CDM.

## PTY i pomiar

Skrypt startuje osobną usługę i TUI na syntetycznej starej kolejce z duplikatami. Sprawdza widoczny ekran, menu i Failed, clear/undo, edycję drugiego klienta i konflikt rewizji, idempotentne ponowienie, uporządkowany zapis/kopię/restart bez autoplay, cztery rozmiary terminala i odtworzenie ECHO/ICANON po q. Pyte interpretuje sekwencje kursora; nie wyszukujemy pełnych wierszy w surowym strumieniu Ratatui. Nie zachowujemy surowego terminala ani danych konta.

Pomiar obejmuje TUI, usługę i wszystkie ich potomne procesy wykryte przez `/proc/*/task/*/children`; prywatny D-Bus oraz sam skrypt są narzędziami testu, poza kosztem aplikacji. Pomiar: **RSS 17 720 KiB (17,30 MiB), PSS 8 470 KiB (8,27 MiB), CPU 0,0% w 2,000081 s**. Dwa własne procesy, 0 zakończonych podczas próbki, 0 procesów audio i 0 przeglądarki. Surowe liczby i skróty badanych plików są w `2026-11-10-albums-queue-pty.json`. 0,0% oznacza brak przyrostu ticków CPU w tym oknie i rozdzielczości zegara, a nie gwarancję zerowego zużycia. To syntetyczny stan zatrzymany, nie koszt aktywnego audio/CDM.

## Przegląd i granice dowodu

Niezależny agent przejrzał bieżący diff i nowe moduły. Wszystkie pięć istotnych ustaleń poprawiono z testami regresyjnymi: autoplay po usunięciu zatrzymanego current, rozjazd pamięci/pliku po fsync, limit fallbacku, zwykłe Previous i źródło szczegółów albumu. Końcowa kontrola autora objęła dodatkowe wyścigi EOF/Stop/anulowania oraz zachowanie istniejącego odświeżania tekstów.

Nie uruchamiano prawdziwego odtwarzania Apple, prywatnego CDM ani autoryzacji subskrybowanego konta. Kompletny syntetyczny album i publiczne metadane nie stanowią dowodu odsłuchu prawdziwego albumu. Natywny backend nadal wymaga lokalnych poświadczeń użytkownika i zgodnego CDM; błędy mają Failed, bez browser fallback i cichego pomijania.

Limity iTunes/HTTP oraz budżety są jawne. Przekroczenie limitu, 429/awaria strony lub niepewna kompletność kończy przygotowanie bez commitu; można ponowić później. Deduplikacja trwa przez ostatnie 64 zatwierdzone klucze, Undo przechowuje jedną sesję tylko do końca procesu. Po błędzie synchronizacji katalogu już po rename edycje są blokowane do restartu, bez obietnicy trwałości po zaniku zasilania. Nie badano zaniku zasilania ani rzeczywistego pełnego dysku; kontrolowane błędy pokrywają granice zapisu/rename/fsync.

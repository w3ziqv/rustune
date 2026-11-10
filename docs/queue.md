# Albumy i sesja kolejki

Usługa Rustune jest właścicielem sesji. TUI, CLI i MPRIS wysyłają intencje i pokazują potwierdzony model usługi. Wybór wiersza, otwarcie albumu oraz podgląd tekstu nie uruchamiają audio.

## Akcje w TUI

Przy wyniku albumu Enter otwiera utwory, a widoczne w nagłówku **a — Album actions** otwiera menu całego albumu. To samo menu działa w szczegółach albumu:

| Akcja | Skutek |
|---|---|
| Play album | Pobranie wszystkich stron, zastąpienie kolejki, start od pierwszej płyty/ścieżki; shuffle wyłączone |
| Shuffle album | Pełny album jako sesja; losowy pierwszy utwór i jedna stabilna permutacja |
| Play album next | Cały album jako kolejny blok FIFO; obecny dźwięk i pozycja bez zmian |
| Append whole album | Cały album na końcu; bez automatycznego startu |

Enter na utworze otwiera osobne akcje: Lyrics and cover, Append to end, Play song, Play next. Play z widoku albumu zachowuje **pełny album**, również wcześniejsze utwory, i rozpoczyna wybrany utwór. Przy shuffle off dalej gra N+1, przez granice stron i płyt. Play w wynikach wyszukiwania zastępuje kolejkę pojedynczym utworem. Menu ostrzega o zastąpieniu.

Queue pokazuje odrębne zaznaczenie, znacznik bieżącego wpisu, rzeczywistą kolejność nadchodzących `[next N]`, źródło/kontekst, trwały `#entry_id`, shuffle i repeat. Enter → Play uruchamia istniejące wystąpienie. Delete / Remove and advance usuwa konkretne wystąpienie; bieżący wpis przechodzi do następnego ze stanem playing/paused, a po zatrzymaniu pozostaje zatrzymany. J/K lub menu przenoszą wpis w widocznej kolejności. Przeniesienie bieżącego wpisu zachowuje jego tożsamość i audio; dalszy ciąg wynika z jego nowego miejsca. Edycja anuluje nawigację do przodu z historii, aby ręczny porządek był wiążący.

`:clear` usuwa nadchodzące (i zbędną historię), zachowując bieżący wpis, pozycję, pauzę, shuffle i repeat. Po jego EOF usługa się zatrzymuje także przy repeat one/all; nowe dodanie ponownie umożliwia dalszy ciąg. `u` w Queue lub `:undo` przywraca jeden poprzedni snapshot sprzed ostatniego zastąpienia/wyczyszczenia. Audio zostaje zatrzymane; dawnej pozycji i dźwięku nie odtwarzamy. Snapshot ma maksymalnie jedną ograniczoną sesję i nie przetrwa restartu.

`:stop` zatrzymuje audio i zeruje pozycję; `:shutdown` zamyka usługę. `:cancel` / `c` anuluje przygotowywanie albumu przed zapisem. Gdy zaczyna się trwały zapis, anulowanie nie jest już dostępne. `:retry` / `R` ponawia ostatnią operację albumu lub jawne żądanie kolejki. Failed audio pozostaje widoczne (również podczas wpisywania wyszukiwania); Play ponawia bieżący utwór.

## Kolejność, historia i tryby

Każde wystąpienie ma własny trwały liczbowy identyfikator. Duplikaty katalogowego song ID są dozwolone. Usuwanie/przenoszenie/Play używają identyfikatora wystąpienia i rewizji sesji; nieaktualny klient otrzymuje konflikt zamiast operacji na innym wierszu.

Play next X, potem Y daje X → Y → dalszy kontekst. Bloki albumów zachowują wewnętrzną kolejność. Wstawki mają pierwszeństwo przed pozostałą permutacją shuffle i przed powrotem do przodu po Previous. Same dodania do pustej/zatrzymanej kolejki przygotowują sesję bez autoplay.

Shuffle jest zapisaną kolejnością, a nie losowaniem przy każdym EOF. Zmiana shuffle dotyczy przyszłego fragmentu, pozostawia bieżący utwór i blok next. Wyłączenie shuffle przywraca kolejność źródłową/chronologię dodawania przyszłych wpisów. Append zachowuje stabilną kolejność dopisanych pozycji. Ręczne przenoszenie zmienia plan słyszalnego dalszego ciągu.

Previous przy pozycji **>3 s** przewija bieżący utwór do początku. Przy pozycji **≤3 s** i shuffle off wybiera poprzedni wpis kolejki, a przy shuffle on poprzedni rzeczywiście słyszany wpis. Następne Next wraca przez historię do przodu i zachowuje już ustalony dalszy ciąg. Next/Previous zachowują pauzę. Repeat One dotyczy EOF; ręczne Next przechodzi dalej. Repeat All powtarza zatwierdzoną listę, wraz ze świadomie dodanymi wpisami. Pusta kolejka nie ma kandydata do odtwarzania. Błąd audio zatrzymuje się jako Failed; nie pomijamy go po cichu.

## Pobieranie i transakcja

Referencja albumu zawiera dostawcę (`music` albo `itunes`), storefront, album ID i tytuł. Szczegóły i kolejne strony są przypięte do tego źródła; nie wykonujemy URL-a kursora zwróconego przez serwer. Apple Music stronicuje relację do rzeczywistego końca, również poza 200 wyników. Zwykłe wyszukiwanie nadal ma limit 200. Fallback iTunes ma ograniczony lookup; pełny album akceptujemy tylko wtedy, gdy `trackCount` potwierdza kompletność odpowiedzi. Większy/niekompletny fallback wymaga ponowienia ze źródłem Apple Music.

Wiarygodne discNumber/trackNumber ustalają kolejność tylko wtedy, gdy ma je cały przygotowany album. Przy brakach zachowujemy kolejność relacji. Niedostępne, nieobsługiwane lub nieprawidłowe zasoby mają podaną liczbę i przyczynę pominięcia. Błąd strony, zmiana deklarowanej liczby rekordów, limit lub anulowanie nie zatwierdzają części albumu i nie przerywają starego audio. Album bez odtwarzalnych utworów zostaje odrzucony.

Etapy operacji są oddzielne: `preparing` (postęp pobierania), `saving` (walidacja/zapis), `committed` (nowa kolejka), następnie audio `Preparing` → potwierdzone `Playing`/`Paused` albo `Failed`. Ready potwierdza przygotowanie backendu; samo przyjęcie kolejki nie oznacza płynącego audio. Generacje izolują stare pobrania i stare eventy workera.

Przygotowanie działa poza silnikiem. Commit sprawdza pierwotną rewizję i wykonuje jeden uporządkowany zapis; nowsza edycja powoduje konflikt. Jeden writer stanu i niezależny writer konfiguracji wykonują pracę dyskową poza pętlą sterowania. W czasie zapisu inne edycje kolejki zwracają zajętość; Pause, Volume, Status i subskrypcja pozostają dostępne. TUI ma oddzielne ograniczone kanały metadanych, mutacji i transportu.

Klucz żądania jest zapisywany wraz z dokładnymi argumentami i rewizją. Ponowienie tego samego klucza nie dubluje operacji ani nie uruchamia ponownie już zatwierdzonego albumu. Inne argumenty z tym samym kluczem są odrzucane. Nowe świadome dodanie używa nowego klucza. Deduplikacja obejmuje ostatnie **64** zatwierdzone operacje, również po restarcie; starszych kluczy nie należy ponawiać. Anulowanie i ponowienie niezakończonej operacji zachowuje klucz, ale dostaje nową generację pobierania.

Limity: 10 000 wpisów (lub niższe `max_queue`), 12 MiB przygotowanych metadanych albumu, 16 MiB całego zapisu, 64 KiB żądania IPC, 4 MiB ramki odpowiedzi. Album przesyłany jest referencją, a QueuePage stronicuje wpisy; nie przesyłamy wielkiej listy przez serię Enqueue. Ostateczna walidacja obejmuje narzut wpisów, kontekstów, historii i potwierdzeń idempotencji. Akceptowany plik ma rozmiar ≤16 MiB; przekroczenie o jeden bajt zostaje odrzucone przed podmianą.

## Migracja i odzyskiwanie

Stary `state.json` bez wersji jest odczytywany z zachowaniem kolejności, current, shuffle, repeat i wszystkich duplikatów. Brakujące tożsamości wystąpień zostają przypisane deterministycznie. Pierwsza udana edycja zapisuje format **version 2** i prywatny `state.previous.json`. Odzyskanie kolejki nigdy nie uruchamia muzyki. Przy `restore_queue=false` startuje pusta sesja, zachowując przestrzeń tożsamości, rewizję, preferencje i ochronę niepoprawnego pliku. Działają także kolejki legacy blisko granicy rozmiaru: pozostają w pamięci, a nowy zapis musi zmieścić się wraz z nowym narzutem.

Nieprawidłowego/za dużego/nowszego pliku nie nadpisujemy przy następnej edycji. Status pokazuje recovery required. `rustune ctl recover-state` zachowuje oryginał jako prywatny `state.recovery-<id>.json` i odblokowuje nową sesję; poprzedni poprawny zapis pozostaje w `state.previous.json`. Aby odzyskać poprzednią sesję, zamknij usługę, zachowaj wadliwy plik i przywróć wybraną poprawną kopię jako `state.json`. Żadne poświadczenia nie są częścią tych plików.

Jeśli podmiana pliku już nastąpiła, ale synchronizacja katalogu zawiodła, nie traktujemy tego jako odrzucenia przed zapisem. Usługa przyjmuje zainstalowaną kolejkę i potwierdzenia do pamięci, zatrzymuje audio oraz blokuje edycje. Komunikat informuje o niepewnej trwałości. Restart ponownie odczytuje plik bez autoplay; recover-state nie przemianowuje tego poprawnego pliku jako uszkodzonego.

## CLI i IPC

```sh
rustune album 123 --action play --storefront pl
rustune album 123 --action shuffle --storefront pl
rustune album 123 --action next --provider music --storefront pl
rustune album 123 --action append --key my-unique-request --revision 42
rustune album 123 --action play --start-song 456 --storefront pl
rustune song 456 --action play
rustune song 456 --action next
rustune queue list             # opcjonalny drugi argument: offset strony
rustune queue play 17          # identyfikator wystąpienia z QueuePage
rustune queue remove 17
rustune queue up 17
rustune queue clear-upcoming
rustune queue undo
rustune ctl cancel OPERATION_ID
rustune ctl retry
rustune ctl stop
rustune ctl shutdown
```

Bez `--revision` CLI odczytuje aktualną rewizję; podanie jej pozwala wykryć zmianę od własnego wcześniejszego odczytu. `--key` umożliwia bezpieczne ponowienie po utracie odpowiedzi. Album zwraca przyjętą operację przygotowania; jej postęp/wynik jest w Status/Subscribe. Jawna QueueAction czeka na wynik zapisu. Odpowiedź kolejki może zawierać Failed audio: zapis i audio mają odrębne potwierdzenia. Nowi klienci używają `QueueAction` oraz `Album`; stare niekontrolowane Remove/Move/PlayIndex zwracają instrukcję migracji. Stare Enqueue jest adapterem do jawnych intencji, a nie implementacją albumów.

MPRIS Stop zatrzymuje audio, Quit kończy usługę, track ID identyfikuje wystąpienie, a możliwości Next/Previous i status wynikają z sesji. Zmiany stanu/metadanych/możliwości są sygnalizowane. Nie dodano browser playback, logowania przez przeglądarkę ani obchodzenia DRM.

## Weryfikacja

Patrz `docs/validation/2026-11-10-albums-queue.md`. Fixtures HTTP, sterowany backend i PTY nie potwierdzają autoryzacji Apple, działania CDM ani odsłuchu prawdziwego kompletnego albumu. Ta praca nie zmienia instalacji użytkownika ani nie publikuje wersji.

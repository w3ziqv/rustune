# Playlisty Apple Music

Playlisty korzystają z lokalnie zapisanego Music User Token oraz anonimowo pobieranego tokenu aplikacji Apple. Ich akceptację sprawdza API; samo zapisanie tokenu nie oznacza autoryzacji ani dostępności natywnego silnika DRM. Nie uruchamiamy przeglądarki. Utworzenie i edycja wymagają dostępu konta do właściwych endpointów Apple Music.

Wybierz **Playlists** albo wpisz `:playlists`. Enter otwiera utwory bez zmiany kolejki. PgDn/PgUp przewijają strony, F5 ponownie pobiera pierwszy ekran. Znacznik `[editable]`/`[read-only]` pochodzi z `canEdit`. Lista i relacja utworów są pobierane na świeżo; obowiązuje budżet 20 żądań na minutę i Retry-After.

`a` otwiera akcje całej playlisty: Play, Shuffle, Play next i Append. Play zastępuje kolejkę; next/append dodają jeden cały blok bez autoplay. Pobieramy wszystkie strony do końca, maksymalnie 10000 rekordów/12 MiB. Błąd, limit, anulowanie lub zmiana lokalnych danych konta nie zatwierdzają części playlisty. Kolejność playlisty jest kolejnością relacji Apple, bez sortowania według albumu. Duplikaty pozostają osobnymi wystąpieniami. Play w menu konkretnego utworu zaczyna od jego pozycji źródłowej, również gdy ten sam song ID występuje wcześniej. Nieodtwarzalne zasoby są pomijane z podaniem przyczyny. Odtwarzanie używa istniejącego natywnego backendu i jego rzeczywistych ograniczeń.

Zapisy na koncie:

- `N` w Playlists / `:playlist-new`: nowa pusta prywatna playlista.
- `S` w Playlists lub Queue / `:playlist-save`: cała widoczna kolejka jako nowa prywatna playlista, w jej kolejności, wraz z duplikatami.
- `A` / ostatnia akcja menu playlisty: dopisanie całej kolejki do wybranej edytowalnej playlisty.
- Enter na utworze → **Add song to Apple playlist…**: wybór edytowalnej playlisty docelowej i dopisanie tego utworu. W Queue wybierane jest konkretne wystąpienie.

Nowa playlista wymaga nazwy (maksymalnie 200 bajtów UTF-8), potem osobnego potwierdzenia **Enter SEND to Apple**. Dopisanie również ma potwierdzenie. Esc anuluje przed wysłaniem. Usługa sprawdza rewizję kolejki oraz aktualne `canEdit` na serwerze. Zapis konta działa niezależnie od transportu, nie zmienia kolejki ani audio. Jeden zapis obejmuje maksymalnie 1000 utworów/128 KiB w jednym POST; większy lub nieprawidłowy snapshot jest odrzucany w całości. Ograniczenie dotyczy eksportu, nie przeglądania ani przygotowania odtwarzania.

Status `playlist_write` rozróżnia `preparing`, `completed`, `rejected`, `uncertain`. Completed oznacza potwierdzoną odpowiedź Apple, nie ukończone odtwarzanie. `:playlist-retry` / `rustune playlist retry` ponawia wyłącznie zdecydowanie odrzuconą operację. Utrata odpowiedzi, HTTP 408/5xx lub niepewny zapis potwierdzenia blokują ponowne wysłanie tego samego klucza. Najpierw sprawdź konto w Apple Music; nową operację z nowym kluczem podejmuj dopiero po ustaleniu wyniku.

Prywatny dziennik `${XDG_DATA_HOME:-~/.local/share}/rustune/playlist-writes.json` (0600, limit 8 MiB/64 operacje) przechowuje argumenty, identyfikatory i potwierdzenia, bez tokenów. Zapis Sending jest synchronizowany przed POST; blokada wyklucza równoległych writerów także przy dwóch usługach. Completed odtwarza lokalny wynik bez drugiego POST, również po restarcie; nie ponawiaj kluczy starszych niż retencja. Sending/Uncertain nie są automatycznie usuwane przy przepełnieniu. Zmiana pliku credentials blokuje replay pod innym lokalnym kontem. Uszkodzony dziennik zostaje zachowany i blokuje zapisy. Nie usuwaj go jako sposobu automatycznego ponowienia.

CLI (jawne komendy zapisu zmieniają konto bez dodatkowego pytania):

```sh
rustune playlist list
rustune playlist list --offset 25 --storefront pl
rustune playlist tracks p.EXAMPLE --storefront pl
rustune playlist play p.EXAMPLE --storefront pl --from-position 26
rustune playlist play p.EXAMPLE --shuffle
rustune playlist next p.EXAMPLE
rustune playlist append p.EXAMPLE
rustune playlist create 'Nowa pusta' --key create-example
rustune playlist save 'Moja kolejka' --revision 42 --key save-example
rustune playlist add p.EXAMPLE --storefront pl --song 123456 --key add-example
rustune playlist add p.EXAMPLE --entry-id 7 --revision 42 --key add-entry-example
rustune playlist add p.EXAMPLE --revision 42 --key add-queue-example
rustune playlist retry
rustune status
```

`--from-position` liczy od zera przed pominięciami. Bez `--song`/`--entry-id` Add dopisuje kolejkę. Brak rewizji pobiera bieżącą z usługi; określ ją jawnie dla kontrolowanego snapshotu. Pierwsza lista wybiera region konta; dla dalszych stron i operacji podaj właściwy storefront, jeśli różni się od `catalog_country`. Klucz identyfikuje dokładne argumenty, a nie zamiar ponownego dodania. Odpowiedź IPC potwierdza przyjęcie operacji; wynik asynchroniczny odczytaj w `status`.

Endpointy i semantyka: [lista](https://developer.apple.com/documentation/applemusicapi/get-all-library-playlists), [tworzenie](https://developer.apple.com/documentation/applemusicapi/create-a-new-library-playlist), [dodawanie utworów](https://developer.apple.com/documentation/applemusicapi/add-tracks-to-a-library-playlist). Używamy stałego origin API, nagłówków w pamięci, bez cookies i przekierowań. Treści odpowiedzi błędów konta nie trafiają do logów ani komunikatów.

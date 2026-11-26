#!/usr/bin/python3
"""Verify real PTY mouse input with synthetic catalog and queue pages. Requires pyte."""
import argparse
import codecs
import fcntl
import json
import os
import pty
import re
import select
import signal
import socketserver
import struct
import subprocess
import tempfile
import termios
import threading
import time
from pathlib import Path

import pyte

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("binary", type=lambda path: Path(path).resolve())
mode = parser.add_mutually_exclusive_group()
mode.add_argument("--first", action="store_true", help="run only the playlist/resize regression")
mode.add_argument("--benchmark", action="store_true", help="sample resource use during sustained scrolling")
options = parser.parse_args()
binary = options.binary
down = b"\x1b[B"
with tempfile.TemporaryDirectory(prefix="rustune-pty-playlist-") as folder:
    test = Path(folder)
    env = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor")
    for var,name in (("XDG_CONFIG_HOME","c"),("XDG_DATA_HOME","d"),
                     ("XDG_CACHE_HOME","a"),("XDG_RUNTIME_DIR","r")):
        (test/name/"rustune").mkdir(parents=True,mode=0o700)
        env[var] = str(test/name)
    conf = test/"c/rustune"
    (conf/"config.toml").write_text('daemon=true\nartwork=false\nlyrics=false\n[keybindings]\ndown="x"\nup="z"\n')
    (conf/"music-user-token").write_text("synthetic-only")
    (conf/"music-user-token").chmod(0o600)
    endpoint = test/"r/rustune/service.sock"
    def local_generation(runtime_env, socket_path):
        # Let the real daemon derive the binding from the synthetic file's metadata.
        daemon = subprocess.Popen([binary, "daemon"], env=runtime_env,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            for _ in range(100):
                if socket_path.exists():
                    break
                time.sleep(.02)
            initial = json.loads(subprocess.check_output([binary, "status"], env=runtime_env, text=True))
            assert initial["account_generation"] is not None
            return initial
        finally:
            daemon.terminate()
            daemon.wait(timeout=5)

    status = local_generation(env, endpoint)
    generation = status["account_generation"]
    assert not endpoint.exists()

    stop = threading.Event()
    pages = []
    played = []
    requests = []
    catalog_pages = []
    queue_pages = []
    unexpected = []
    toggle_received = threading.Event()
    queue_release = threading.Event()
    playlist_release = {attempt: threading.Event() for attempt in range(1, 5)}
    playlist_finished = {attempt: threading.Event() for attempt in range(1, 5)}
    queue_length = 10_000 if options.benchmark else 41
    status.update(queue_len=queue_length, queue_revision=7, profile="minimal")
    cursors = lambda offset: f"music:pl:{generation:016x}:{offset}"

    def tracks(offset, length, queue=False):
        boundary = 15 if queue else 25
        records = []
        for index in range(offset, min(offset + boundary, length)):
            song = index if index in (boundary, 2 * boundary) else index + 1
            item = {"id": str(song), "title": f"Track {song}", "kind": "library-songs",
                    "provider": "music", "storefront": "pl", "source_index": index}
            if queue:
                item["entry_id"] = 1000 + index
            records.append(item)
        return records

    class Service(socketserver.StreamRequestHandler):
        def handle(self):
            length = self.rfile.read(4)
            if not length:
                return
            request = json.loads(self.rfile.read(struct.unpack(">I", length)[0]))
            command = request["command"]
            args = request.get("args")
            requests.append(request)
            answer = {"ok": True}
            if command == "browse":
                kind = args["kind"]
                cursor = args.get("next")
                offset = 0 if cursor is None else int(cursor.rsplit(":", 1)[1])
                private = kind in ("library", "playlists", "search:library-songs") or kind.startswith("open:library-playlists:")
                page_cursor = cursors if private else lambda offset: f"itunes:pl:{offset}"
                if kind == "open:library-playlists:p.fixture":
                    pages.append(offset)
                    if not options.benchmark and not options.first and offset == 50:
                        attempt = pages.count(offset)
                        if attempt in playlist_release:
                            assert playlist_release[attempt].wait(15), "playlist release not signaled"
                        if attempt == 1:
                            self.frame({"ok": False, "data": None, "error": "temporary fixture outage"})
                            return
                    total = 450 if options.benchmark else 56
                    records = tracks(offset, total)
                elif kind == "playlists":
                    catalog_pages.append((kind, offset))
                    records = [
                        {"id": "p.fixture" if i == 0 else f"p.{i}",
                         "title": "Fixture Mix" if i == 0 else f"Playlist {i + 1}",
                         "kind": "library-playlists", "provider": "music", "storefront": "pl",
                         "can_edit": i != 25} for i in range(offset, min(offset + 25, 56))
                    ]
                    total = 56
                else:
                    catalog_pages.append((kind, offset))
                    total = 56
                    label, row_kind = {
                        "search:songs": ("Public song", "songs"),
                        "search:albums": ("Album", "albums"),
                        "search:artists": ("Artist", "artists"),
                        "search:library-songs": ("Saved song", "library-songs"),
                        "library": ("Library song", "library-songs"),
                        "open:artists:1": ("Artist album", "albums"),
                        "open:albums:1": ("Album song", "songs"),
                    }[kind]
                    records = [{"id": str(i + 1), "title": f"{label} {i + 1}", "kind": row_kind,
                                "provider": "music" if private else "itunes", "storefront": "pl"}
                               for i in range(offset, min(offset + 25, total))]
                answer["data"] = {
                    "items": records,
                    "storefront": "pl",
                    "account_generation": generation if private else None,
                    "next": page_cursor(offset + 25) if offset + 25 < total else None,
                    "previous": page_cursor(offset - 25) if offset >= 25 else None,
                }
            elif command == "queue_page":
                offset = args["offset"]
                queue_pages.append(offset)
                if not options.benchmark and offset == 15 and queue_pages.count(offset) == 1:
                    assert queue_release.wait(15), "queue release not signaled"
                answer["data"] = {"items": tracks(offset, status["queue_len"], queue=True),
                                  "revision": status["queue_revision"], "total": status["queue_len"]}
                if offset == 15:
                    # Older daemons ignore the annotated request field.
                    answer["data"]["upcoming"] = list(range(1000, 1000 + status["queue_len"]))
                else:
                    answer["data"]["upcoming_ranks"] = {
                        item["entry_id"]: item["entry_id"] - 999
                        for item in answer["data"]["items"]
                    }
            else:
                if command in ("playlist", "queue_action", "playlist_write", "album"):
                    played.append(request)
                elif command == "toggle":
                    toggle_received.set()
                elif command not in ("subscribe", "status", "record_search"):
                    unexpected.append(command)
                answer["data"] = status
            self.frame(answer)
            if command == "browse" and kind == "open:library-playlists:p.fixture" and offset == 50:
                if not options.benchmark and not options.first and attempt in playlist_finished:
                    playlist_finished[attempt].set()
            if command == "subscribe":
                previous = dict(status)
                while not stop.wait(.025):
                    current = dict(status)
                    if current != previous:
                        self.frame({"ok": True, "data": current})
                        previous = current

        def frame(self,value):
            body = json.dumps(value).encode()
            try:
                self.wfile.write(struct.pack(">I",len(body))+body)
                self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                pass  # Superseded metadata reads may close their socket before this reply.

    server = socketserver.ThreadingUnixStreamServer(str(endpoint),Service)
    server.daemon_threads = True
    thread = threading.Thread(target=server.serve_forever,daemon=True)
    thread.start()
    master,slave = pty.openpty()
    fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack("HHHH",42,120,0,0))
    tui = subprocess.Popen([binary],env=env,stdin=slave,stdout=slave,stderr=slave,start_new_session=True)
    os.close(slave)
    screen = pyte.Screen(120,42)
    stream = pyte.Stream(screen)
    decoder = codecs.getincrementaldecoder("utf8")("replace")
    checks = []

    def drain(seconds=.05):
        end = time.monotonic()+seconds
        while time.monotonic()<end:
            if select.select([master],[],[],min(.025,max(0,end-time.monotonic())))[0]:
                try:
                    stream.feed(decoder.decode(os.read(master,65536)))
                except OSError:
                    return

    def type_keys(keys,count=1):
        for _ in range(count):
            os.write(master,keys)
            drain(.32 if keys == b"\x1b" else .022)

    def expect(label, test):
        end = time.monotonic()+4
        while not test() and time.monotonic()<end:
            drain()
        assert test(), f"{label}\nRecent requests: {requests[-6:]}\n" + "\n".join(screen.display)
        checks.append(label)

    def visible():
        rows = []
        for line in screen.display:
            match = re.search(r"([› ])\s*(\d+)\s+Track (\d+)\b",line)
            if match:
                rows.append(tuple(int(match[i]) for i in (2,3)))
        return rows

    def window(first,last):
        return [r[0] for r in visible()] == list(range(first,last+1))

    def wheel(x, y, down=True):
        return f"\x1b[<{65 if down else 64};{x};{y}M".encode()

    def song_menu():
        return any("SONG ACTIONS" in line for line in screen.display)

    try:
        expect("TUI starts", lambda: any("RUSTUNE" in line for line in screen.display))

        def compact(rows, cols):
            screen.resize(lines=rows, columns=cols)
            fcntl.ioctl(master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
            tui.send_signal(signal.SIGWINCH)
            drain(.2)

        def click(x, y):
            return f"\x1b[<0;{x};{y}M\x1b[<0;{x};{y}m".encode()

        def row(number):
            label = rf"\b{number}\s+Track \d+\b"
            return next(i + 1 for i, line in enumerate(screen.display) if re.search(label, line))

        def line_with(label):
            return next(i + 1 for i, line in enumerate(screen.display) if label in line)

        def first_check():
            type_keys(b"3")
            expect("private playlist list opens", lambda: any("Fixture Mix" in line for line in screen.display))
            type_keys(b"\r")
            expect("initial entries 1–25", lambda: window(1, 25))
            type_keys(b"\x1b[<67;32;12M\x1b[<66;32;12M")
            expect("horizontal wheel leaves the playlist unchanged", lambda: window(1, 25))
            type_keys(b"x", 20)
            expect("second playlist page prefetched", lambda: seen("50 results"))
            type_keys(b"x", 5)
            expect("page boundary keeps adjacent duplicates", lambda: visible()[-2:] == [(25, 25), (26, 25)])
            type_keys(click(32, row(26)))
            expect("click on second duplicate opens Song Actions", song_menu)
            assert "Track 25" in screen.display[line_with("SONG ACTIONS")]
            compact(10, 60)
            expect("short layout still displays the same menu", song_menu)
            type_keys(wheel(30, line_with("SONG ACTIONS")), 8)
            expect("wheel reaches the last option", lambda: any("› Add song" in line for line in screen.display))
            assert not played

            compact(42, 120)
            expect("wide layout still displays the same menu", song_menu)
            type_keys(wheel(24, 33))
            expect("first wheel outside the menu closes it and scrolls", lambda: not song_menu() and window(3, 27))
            assert any(re.search(r"›\s+26\s+Track 25", line) for line in screen.display)
            assert not played
            type_keys(click(32, row(26)))
            expect("click reopens the actions for the second duplicate", song_menu)
            type_keys(down, 2)
            type_keys(b"\r")
            expect("explicit confirmation sends one playback request", lambda: len(played) == 1)
            action = played[0]["args"]["action"]
            assert action["source_index"] == 25 and action["expected_song"] == "25"
            assert played[0]["args"]["account_generation"] == generation
        def seen(text):
            return any(text in line for line in screen.display)

        def matrix():
            global generation
            type_keys(wheel(24, 8), 23)
            expect("third page is held with rows 26–50 visible", lambda: pages == [0, 25, 50] and window(26, 50))
            type_keys(click(32, row(26)))
            expect("second duplicate selected while playlist page is held", song_menu)
            type_keys(wheel(24, 8), 4)
            expect("boundary wheel leaves the menu open while page is pending", song_menu)
            assert pages == [0, 25, 50]
            type_keys(b"Rx")
            expect("menu processes input while fetching", lambda: seen("› Append to end") and song_menu())
            drain(.2)
            expect("R while loading sends no duplicate playlist read", lambda: pages == [0, 25, 50])
            playlist_release[1].set()
            expect("failed playlist page leaves the original menu open", lambda: seen("temporary fixture outage") and song_menu())
            assert "Track 25" in screen.display[line_with("SONG ACTIONS")]
            type_keys(wheel(24, 8), 6)
            assert pages == [0, 25, 50]
            type_keys(b"R")
            expect("R in the menu retries the same cursor once", lambda: pages == [0, 25, 50, 50])
            assert song_menu()
            type_keys(b"Rz")
            expect("menu still processes input while retrying", lambda: seen("› Lyrics") and song_menu())
            drain(.2)
            expect("R during retry sends no further playlist reads", lambda: pages == [0, 25, 50, 50])
            playlist_release[2].set()
            expect("successful retry scrolls one row before closing the menu",
                   lambda: not song_menu() and window(27, 51) and seen("Selected #26 offscreen"))
            assert len(played) == 1
            type_keys(wheel(24, 8), 8)
            expect("continuous playlist reaches its end", lambda: window(32, 56))
            type_keys(wheel(24, 8, False), 31)
            expect("playlist reverses across both boundaries without refetching", lambda: window(1, 25))
            assert all(index != 0 for index in pages[1:])
            assert seen("Selected #26 offscreen")
            type_keys(b"\r")
            expect("Enter reveals the previous selection without opening actions",
                   lambda: not song_menu() and seen("›  26  Track 25"))
            type_keys(b"\r")
            expect("the second Enter opens the original duplicate", song_menu)
            type_keys(b"\x1b")
            expect("Esc preserves the original selection", lambda: not song_menu() and seen("›  26  Track 25"))
            type_keys(b"3")
            expect("return to playlist directory", lambda: seen("Fixture Mix"))
            type_keys(b"\r")
            expect("reopen the original playlist with a new viewport", lambda: window(1, 25))
            type_keys(wheel(24, 8), 26)
            expect("new viewport holds the third-page reply", lambda: pages.count(50) == 3 and window(26, 50))
            type_keys(click(32, row(26)))
            expect("open a menu over the second duplicate before leaving the view", song_menu)
            type_keys(wheel(24, 8))
            type_keys(b"\x1b")
            type_keys(b"1")
            expect("leave playlist while its page is held", lambda: seen("/ Search"))
            type_keys(b"\x1b")
            expect("restored playlist requests third page again", lambda: pages.count(50) == 4 and window(26, 50))
            type_keys(click(32, row(27)))
            expect("fresh menu belongs to a different occurrence", lambda: song_menu() and "Track 27" in screen.display[line_with("SONG ACTIONS")])
            playlist_release[3].set()
            expect("old reply reaches the dropped connection", playlist_finished[3].is_set)
            drain(.12)
            assert song_menu() and not seen("56 results")
            playlist_release[4].set()
            expect("fresh prefetch updates rows without closing the new menu", lambda: seen("56 results") and song_menu())
            assert "Track 27" in screen.display[line_with("SONG ACTIONS")]
            type_keys(b"\x1b")

            type_keys(click(32, row(26)))
            expect("private duplicate selected before account rotation", song_menu)
            token_file = conf / "music-user-token"
            token_file.unlink()
            status["account_generation"] = None
            expect("account rotation closes the menu and clears private rows", lambda: not song_menu() and not visible())
            type_keys(b"\r")
            assert len(played) == 1

            type_keys(b"4")
            expect("Minimal queue first page", lambda: seen("#1014"))
            expect("annotated queue displays one-based ranks 1 and 2",
                   lambda: seen("[next 1] Track 1") and seen("[next 2] Track 2"))
            type_keys(wheel(32, 10), 5)
            expect("only one QueuePage is pending for the boundary", lambda: queue_pages.count(15) == 1)
            type_keys(b" ")
            expect("Toggle reaches IPC before the held QueuePage is released", toggle_received.is_set)
            assert not queue_release.is_set() and queue_pages.count(15) == 1
            queue_release.set()
            expect("queue crosses to page 2 and retains off-page selection", lambda: seen("#1015") and seen("Selected #1 offscreen"))
            expect("legacy queue displays one-based ranks 16 and 17",
                   lambda: seen("[next 16] Track 15") and seen("[next 17] Track 17"))
            type_keys(wheel(32, 10, False))
            expect("queue wheel returns across the boundary", lambda: seen("#1014") and not seen("Selected #1 offscreen"))
            type_keys(wheel(32, 10))
            expect("queue returns to page 2", lambda: seen("#1015"))
            type_keys(click(33, line_with("#1015")))
            expect("queue click opens the second duplicate", song_menu)
            compact(10, 60)
            type_keys(wheel(30, line_with("SONG ACTIONS")), 9)
            expect("queue menu reaches the sixth option", lambda: seen("› Add song"))
            compact(42, 120)
            type_keys(wheel(31, 8))
            expect("queue background wheel closes the menu and advances", lambda: not song_menu() and seen("#1030"))
            assert len(played) == 1
            type_keys(b"\r")
            expect("Enter retrieves off-page queue selection", lambda: seen("#1015") and not song_menu())
            type_keys(b"\r")
            expect("queue menu restored", song_menu)
            type_keys(b"x\r")
            expect("remapped key confirms exact queue duplicate", lambda: len(played) == 2)
            assert played[-1]["args"]["action"] == {"intent": "play_entry", "entry_id": 1015}
            assert played[-1]["args"]["revision"] == 7
            type_keys(click(33, line_with("#1015")))
            expect("queue opens before external replacement", song_menu)
            status.update(queue_revision=8, queue_len=8)
            expect("external shrink closes stale queue menu and clamps page", lambda: not song_menu() and seen("#1007") and not seen("#1015"))
            type_keys(b"\x1b[6~\x1b[5~")
            assert len(played) == 2
            assert {0, 15, 30}.issubset(queue_pages)

            # Restore only the synthetic fixture account for private catalog paging.
            token_file.write_text("synthetic-only")
            token_file.chmod(0o600)
            other_runtime = test / "second-runtime"
            (other_runtime / "rustune").mkdir(parents=True, mode=0o700)
            other_env = dict(env, XDG_RUNTIME_DIR=str(other_runtime))
            generation = local_generation(other_env, other_runtime / "rustune/service.sock")["account_generation"]
            status["account_generation"] = generation
            drain(.15)

            def pages_through(kind, label):
                expect(f"{kind} first page", lambda: seen(f"{label} 25"))
                type_keys(wheel(33, 10))
                expect(f"{kind} crosses forward", lambda: seen(f"{label} 26"))
                type_keys(wheel(33, 10, False))
                expect(f"{kind} crosses backward", lambda: seen(f"{label} 25") and not seen(f"{label} 26"))
                assert [offset for query, offset in catalog_pages if query == kind][-3:] == [0, 25, 0]

            def search(category):
                type_keys(b"1")
                expect("starting a new search returns to Search", lambda: seen("SEARCH  songs"))
                type_keys(b"/")
                expect("search input captures text", lambda: seen("Tab type"))
                type_keys(b"\t", category)
                label = ["songs", "albums", "artists", "My library"][category]
                expect(f"input selects {label}", lambda: seen(f"SEARCH  {label}"))
                type_keys(b"fixture\r")

            for category, kind, label in [
                (0, "search:songs", "Public song"), (1, "search:albums", "Album"),
                (2, "search:artists", "Artist"), (3, "search:library-songs", "Saved song"),
            ]:
                search(category)
                pages_through(kind, label)
                if category == 2:
                    type_keys(b"\r")
                    pages_through("open:artists:1", "Artist album")
                    type_keys(b"\r")
                    pages_through("open:albums:1", "Album song")
                    type_keys(b"\x1b")
                    expect("artist albums restored with their selection", lambda: seen("ARTIST ALBUMS") and seen("Artist album 1"))
                    type_keys(b"a")
                    expect("Album Actions opens", lambda: seen("ALBUM ACTIONS"))
                    compact(7, 60)
                    type_keys(wheel(30, line_with("ALBUM ACTIONS")), 5)
                    expect("clipped Album Actions reaches its last action", lambda: seen("› Append whole album"))
                    type_keys(wheel(30, line_with("ALBUM ACTIONS"), False), 5)
                    expect("Album Actions returns to the first action", lambda: seen("› Play album"))
                    type_keys(b"\x1b")
                    expect("Esc closes Album Actions", lambda: not seen("ALBUM ACTIONS"))
                    compact(42, 120)

            type_keys(b"2")
            pages_through("library", "Library song")
            type_keys(b"3")
            pages_through("playlists", "Playlist")
            type_keys(b"a")
            expect("Playlist Actions opens", lambda: seen("PLAYLIST ACTIONS"))
            compact(7, 60)
            type_keys(wheel(30, line_with("PLAYLIST ACTIONS")), 6)
            expect("clipped Playlist Actions reaches its last action", lambda: seen("› Add current queue"))
            type_keys(wheel(30, line_with("PLAYLIST ACTIONS"), False), 6)
            expect("Playlist Actions returns to the first action", lambda: seen("› Play playlist"))
            type_keys(b"\x1b")
            expect("Esc closes Playlist Actions", lambda: not seen("PLAYLIST ACTIONS"))
            type_keys(b"N")
            expect("create form opens", lambda: seen("NEW APPLE MUSIC PLAYLIST"))
            type_keys(b"private test title")
            expect("the edit cursor remains visible", lambda: seen("private test title▏"))
            type_keys(wheel(30, 4, False), 10)
            expect("creation form wheel reaches instructions", lambda: seen("Playlist name"))
            type_keys(wheel(30, 4), 10)
            expect("creation form wheel returns to edit text", lambda: seen("private test title▏"))
            type_keys(b"\r")
            expect("name Enter only opens confirmation", lambda: seen("CONFIRM ACCOUNT WRITE"))
            type_keys(wheel(30, 4), 12)
            expect("confirmation wheel reaches final warning", lambda: seen("new operation"))
            type_keys(wheel(30, 4, False), 12)
            expect("confirmation wheel reaches the explicit prompt", lambda: seen("Enter SEND to Apple"))
            type_keys(b"\x1b")
            expect("Esc cancels without writing", lambda: not seen("CONFIRM ACCOUNT WRITE"))
            type_keys(b"L")
            expect("login prompt opens", lambda: seen("LOGIN"))
            type_keys(b"fixture-token-that-must-stay-hidden")
            expect("login shows masked feedback", lambda: seen("characters)"))
            assert not seen("fixture-token-that-must-stay-hidden")
            type_keys(wheel(30, 4, False), 8)
            expect("login wheel reaches instructions", lambda: seen("Paste token (hidden)"))
            type_keys(wheel(30, 4), 8)
            expect("login wheel returns to masked feedback", lambda: seen("characters)"))
            type_keys(b"\x1b")
            assert token_file.read_text() == "synthetic-only"

            compact(42, 120)
            type_keys(b"3")
            expect("playlist source opens for destination flow", lambda: seen("Fixture Mix"))
            type_keys(b"\r")
            expect("playlist tracks open for destination flow", lambda: window(1, 25))
            type_keys(click(32, row(1)))
            expect("destination flow opens Song Actions", song_menu)
            y = line_with("SONG ACTIONS")
            type_keys(wheel(60, y), 5)
            type_keys(b"\r")
            expect("song action opens destination chooser", lambda: seen("CHOOSE EDITABLE DESTINATION") and seen("Playlist 25"))
            type_keys(wheel(34, 9))
            expect("destination chooser pages forward", lambda: seen("Playlist 26 [read-only]"))
            type_keys(b"x\r")
            expect("read-only target remains read-only", lambda: seen("This playlist is read-only"))
            type_keys(b"x\r")
            expect("editable target still requires confirmation", lambda: seen("CONFIRM ACCOUNT WRITE"))
            type_keys(b"\x1b")

            compact(12, 140)
            type_keys(b"1/fixture\r")
            expect("search returns in laid-out side panels", lambda: seen("Public song 1"))
            rows_before = [line[20:111] for line in screen.display[5:8]]
            type_keys(wheel(9, 5), 10)
            expect("short left panel reaches Settings", lambda: any("Settings" in line[:18] for line in screen.display))
            type_keys(wheel(9, 5, False), 10)
            expect("left panel returns to Search", lambda: any("Search" in line[:18] for line in screen.display))
            type_keys(wheel(125, 5), 28)
            expect("right panel reaches last details", lambda: seen("Choose Play for"))
            type_keys(wheel(125, 5, False), 28)
            expect("right panel returns to its heading", lambda: seen("APPLE MUSIC"))
            assert [line[20:111] for line in screen.display[5:8]] == rows_before
            assert [offset for kind, offset in catalog_pages if kind == "search:songs"][-1] == 0

            compact(12, 70)
            type_keys(b"?")
            expect("short Help opens", lambda: seen("HELP · YOUR BINDINGS"))
            type_keys(wheel(30, 6), 50)
            expect("Help wheel reaches the last wrapped lines", lambda: seen("customizes") and seen("bindings."))
            type_keys(wheel(30, 6, False), 50)
            expect("Help wheel returns to its start", lambda: seen("HELP · YOUR BINDINGS"))
            type_keys(b"e")
            expect("compact equalizer opens", lambda: seen("EQUALIZER"))
            type_keys(wheel(30, 7), 14)
            expect("equalizer scroll reaches preamp without adjustment", lambda: seen("› Preamp"))
            type_keys(wheel(30, 7, False), 14)
            expect("equalizer scroll returns to Enabled", lambda: seen("› Enabled"))
            type_keys(b"7")
            expect("short Settings opens", lambda: seen("SETTINGS"))
            type_keys(wheel(30, 7), 18)
            expect("settings wheel reaches Quit without acting", lambda: seen("  Quit"))
            type_keys(wheel(30, 7, False), 18)
            expect("settings wheel returns to token row", lambda: seen("Music User Token"))
            type_keys(b":audio\r")
            expect("audio information opens", lambda: seen("AUDIO INFORMATION"))
            type_keys(wheel(30, 7), 35)
            expect("audio information reaches last line", lambda: seen("Source bit depth unknown"))
            type_keys(wheel(30, 7, False), 35)
            expect("audio information scroll returns to first checks", lambda: seen("User Token"))
            type_keys(b"R")
            compact(4, 18)
            expect("tiny terminal asks to enlarge", lambda: seen("Enlarge terminal"))
            type_keys(wheel(8, 3), 3)
            assert len(played) == 2 and not unexpected, (played, unexpected)
            compact(12, 70)

        def resource_sample():
            stat = Path(f"/proc/{tui.pid}/stat").read_text().rsplit(")", 1)[1].split()
            memory = {}
            for line in Path(f"/proc/{tui.pid}/smaps_rollup").read_text().splitlines():
                key, _, value = line.partition(":")
                if key in ("Rss", "Pss"):
                    memory[key.lower() + "_kib"] = int(value.split()[0])
            return memory, (int(stat[11]) + int(stat[12])) / os.sysconf("SC_CLK_TCK")

        def benchmark(select_view, label, ipc_trace):
            type_keys(select_view)
            expect(f"{label} loaded", lambda: seen("Track 1"))
            start = time.monotonic()
            baseline, cpu_start = resource_sample()
            samples = [baseline]
            for move_down in (True, False):
                for _ in range(12):
                    type_keys(wheel(32, 10, move_down), 10)
                    samples.append(resource_sample()[0])
            after, cpu_end = resource_sample()
            return {"seconds": round(time.monotonic() - start, 3), "wheel_events": 240,
                    "cpu_seconds": round(cpu_end - cpu_start, 3), "rows_requests": len(ipc_trace),
                    "unique_offsets": len(set(ipc_trace)),
                    "rss_peak_kib": max(item["rss_kib"] for item in samples),
                    "pss_peak_kib": max(item["pss_kib"] for item in samples)}

        benchmarks = {}
        if options.benchmark:
            type_keys(b"3")
            expect("playlist selector ready", lambda: seen("Fixture Mix"))
            benchmarks["playlist_450"] = benchmark(b"\r", "playlist", pages)
            benchmarks["queue_10000"] = benchmark(b"4", "queue", queue_pages)
            assert not played and not unexpected
        else:
            first_check()
            if not options.first:
                matrix()
        type_keys(b"q")
        assert tui.wait(timeout=5)==0
    finally:
        if tui.poll() is None:
            tui.terminate()
            tui.wait(timeout=5)
        os.close(master)
        stop.set()
        queue_release.set()
        for gate in playlist_release.values():
            gate.set()
        server.shutdown()
        server.server_close()

    print(json.dumps({"checks": checks, "benchmarks": benchmarks, "metadata": "synthetic", "audio": False},
                     ensure_ascii=False, indent=2))

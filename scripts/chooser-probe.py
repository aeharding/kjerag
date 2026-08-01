#!/usr/bin/env python3
"""What would the file chooser hand this app? (issue #123)

    scripts/chooser-probe.py dev.harding.Kjerag <file> [file...]

The question a Flatpak cannot ask from the inside. When a sandboxed app picks
a file, xdg-desktop-portal does not hand back what the dialog returned: it
passes the file to the document portal first, with
REUSE_EXISTING|PERSISTENT|AS_NEEDED_BY_APP and the permissions its own flags
produced (`src/file-chooser.c` send_response, `src/documents.c`
register_document, 1.18.4). An empty doc id back means "the app can already
reach this", and the chooser then answers with the real path; any other doc
id means the app is answered with `<mountpoint>/<id>/<name>`, a directory
holding that one file, which is why a capture written as two files plays one
lens when it is picked there.

This makes that same call, so the answer can be read with no dialog to click.
It makes it twice per file, because which one happens is not about the app's
grants alone:

  write asked    what a backend that sends no `writable` result produces,
                 which is xdg-desktop-portal-cosmic always and
                 xdg-desktop-portal-gtk unless the pilot ticks "Open files
                 read-only". A read-only grant does not satisfy it.
  read only      what those same backends produce with the box ticked. A
                 read-only grant satisfies this, and the real path comes back.

So it is the pairing of the grant and the request that decides, and the same
file can answer both ways. Run it on a file inside a grant and on one outside
to see the instrument produce both answers.

It registers documents as a side effect, exactly as the chooser does. They are
listed by `flatpak documents` and removed with `flatpak document-unexport`.
"""

import os
import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

# DOCUMENT_ADD_FLAGS_REUSE_EXISTING | _PERSISTENT | _AS_NEEDED_BY_APP.
FLAGS = 1 | 2 | 4

REQUESTS = (
    ("write asked", ["read", "write", "grant-permissions"]),
    ("read only ", ["read", "grant-permissions"]),
)

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)


def call(method, params, reply):
    return bus.call_sync(
        "org.freedesktop.portal.Documents",
        "/org/freedesktop/portal/documents",
        "org.freedesktop.portal.Documents",
        method,
        params,
        GLib.VariantType(reply),
        Gio.DBusCallFlags.NONE,
        -1,
        None,
    )


def mountpoint():
    (raw,) = call("GetMountPoint", None, "(ay)").unpack()
    return bytes(raw).rstrip(b"\0").decode()


def add_full(path, app_id, permissions):
    fds = Gio.UnixFDList.new()
    fd = os.open(path, os.O_PATH | os.O_CLOEXEC)
    index = fds.append(fd)
    os.close(fd)
    result, _ = bus.call_with_unix_fd_list_sync(
        "org.freedesktop.portal.Documents",
        "/org/freedesktop/portal/documents",
        "org.freedesktop.portal.Documents",
        "AddFull",
        GLib.Variant("(ahusas)", ([index], FLAGS, app_id, permissions)),
        GLib.VariantType("(asa{sv})"),
        Gio.DBusCallFlags.NONE,
        -1,
        fds,
        None,
    )
    (ids, _extra) = result.unpack()
    return ids[0] if ids else ""


def main():
    if len(sys.argv) < 3:
        print(__doc__.splitlines()[2].strip(), file=sys.stderr)
        return 2
    app_id, paths = sys.argv[1], sys.argv[2:]
    mount = mountpoint()
    print(f"app        {app_id}")
    print(f"mountpoint {mount}")
    for path in paths:
        print(f"\npicked     {path}")
        for label, permissions in REQUESTS:
            doc = add_full(path, app_id, permissions)
            real = doc == ""
            handed = path if real else os.path.join(mount, doc, os.path.basename(path))
            answer = "the real path" if real else f"a document, id {doc}"
            print(f"  {label}  {answer}")
            print(f"              file://{handed}")
    return 0


sys.exit(main())

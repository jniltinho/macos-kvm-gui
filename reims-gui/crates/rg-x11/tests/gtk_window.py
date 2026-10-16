# Test helper: a real GTK3 window titled by argv[1], like QEMU's -display gtk window.
import sys

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk  # noqa: E402

w = Gtk.Window(title=sys.argv[1])
w.set_default_size(320, 240)
w.connect("destroy", Gtk.main_quit)
w.show_all()
Gtk.main()

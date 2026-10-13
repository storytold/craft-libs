# craft-clipboard

Native Wayland MIME clipboard access on the application's owned winit connection. A worker
uses winit's existing focused data device and selection offers, and services data sources on its own event queue. This supports
image, file and rich-text formats without a data-control extension or an X server. X11 and
other operating systems retain each app's existing clipboard implementation.

The worker owns immutable bytes per source, checks keyboard focus and selection serials,
and services nonblocking pipes with byte, transfer and time limits. Requests use a bounded
channel and have a finite deadline. The last handle wakes the worker to shut down; no permanent
polling timer or hidden window is needed. The worker wakes the main event loop before coordinated shared-display reads and after dispatch; it does not request redraws. App codecs remain responsible for decoding image data.

This implementation's seat-focus pattern follows smithay-clipboard 0.7.3, licensed MIT:
https://github.com/smithay/smithay-clipboard. The upstream MIT notice is preserved in
LICENSE-smithay-clipboard. The protocol implementation uses smithay-client-toolkit directly.

Runtime and error-path qualification is still required before release.

Copied-file reception supports local URI lists and portal file-transfer keys. Portal retrieval uses
a five-second total deadline and validates all returned paths before exposing any. It prefers portal
paths for sandboxed receivers. This grants only the access provided by the document portal.

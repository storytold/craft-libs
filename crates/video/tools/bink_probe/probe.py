"""Black-box probe harness: build one-frame Bink files and decode them with ffmpeg."""
import struct, subprocess, os, sys, tempfile

def sample_path():
    """The sample the probes are built on: a 180x139 revision-i file whose frames 0 and 1 are black (BINK_PROBE_SAMPLE)."""
    p = os.environ.get('BINK_PROBE_SAMPLE')
    if not p: raise SystemExit('set BINK_PROBE_SAMPLE to the path of the probe sample (see README.md)')
    return p

def parse(path):
    d = open(path, 'rb').read()
    size, frames, largest, frames2, w, h, fn, fd, flags, ntr = struct.unpack_from('<10I', d, 4)
    p = 44 + 12 * ntr
    idx = struct.unpack_from('<%dI' % (frames + 1), d, p)
    return dict(d=d, sig=d[:4], frames=frames, w=w, h=h, fn=fn, fd=fd, flags=flags, ntr=ntr, idx=idx)

def video_packet(f, i):
    o = f['idx'][i] & ~1; e = f['idx'][i + 1] & ~1; q = o
    for t in range(f['ntr']):
        L, = struct.unpack_from('<I', f['d'], q); q += 4 + L
    return f['d'][q:e]

def build(sig, w, h, packets, flags=0, keys=None):
    """A Bink file with no audio and the given video packets."""
    n = len(packets)
    start = 44 + 4 * (n + 1)
    packets = [p + b'\0' * (-len(p) % 4) for p in packets]
    total = start + sum(map(len, packets))
    out = bytearray(sig)
    out += struct.pack('<10I', total - 8, n, max(map(len, packets)), n, w, h, 25, 1, flags, 0)
    at = start
    for k, p in enumerate(packets):
        key = (k == 0) if keys is None else keys[k]
        out += struct.pack('<I', at | int(key)); at += len(p)
    out += struct.pack('<I', at)
    for p in packets: out += p
    return bytes(out)

TMP = tempfile.mkdtemp(prefix='binkprobe')
import itertools
_COUNTER = itertools.count()
def decode(bik, frame=None):
    """Decode with ffmpeg; returns (yuv420p bytes of all frames, stderr)."""
    import threading, itertools
    path = os.path.join(TMP, 'p%d_%d_%d.bik' % (os.getpid(), threading.get_ident(), next(_COUNTER)))
    open(path, 'wb').write(bik)
    r = subprocess.run(['ffmpeg', '-v', 'error', '-i', path, '-f', 'rawvideo', '-pix_fmt', 'yuv420p', '-fps_mode', 'passthrough', '-'], capture_output=True)
    os.remove(path)
    return r.stdout, r.stderr.decode(errors='replace')

def planes(yuv, w, h, i=0):
    cw, ch = (w + 1) // 2, (h + 1) // 2
    fs = w * h + 2 * cw * ch; f = yuv[i * fs:(i + 1) * fs]
    return f[:w * h], f[w * h:w * h + cw * ch], f[w * h + cw * ch:]

if __name__ == '__main__':
    f = parse(sys.argv[1]); w, h = f['w'], f['h']
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    ref, err = decode(f['d'])
    pk = [video_packet(f, i) for i in range(n)]
    got, err2 = decode(build(f['sig'], w, h, pk, f['flags']))
    fs = w * h + 2 * ((w + 1) // 2) * ((h + 1) // 2)
    print('ref frames', len(ref) // fs, 'rebuilt frames', len(got) // fs, 'match', got == ref[:n * fs], repr(err[:200]), repr(err2[:200]))
    print('first words of frame 0 video packet:', [hex(x) for x in struct.unpack_from('<6I', pk[0], 0)], 'len', len(pk[0]))

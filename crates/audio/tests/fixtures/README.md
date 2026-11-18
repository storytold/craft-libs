Tiny fixtures for `tests/audio/decode_extensions.rs`: one per container or codec that
`DECODE_EXTENSIONS` names and the crate cannot write itself. All are a 0.1 s mono 440 Hz sine
from ffmpeg's `sine` generator (`F="-y -f lavfi -i sine=frequency=440:duration=0.1 -ac 1"`).

From SoundCraft (`crates/audio-io/tests/fixtures`), made with ffmpeg 8.0:

    ffmpeg $F -ar 8000  -c:a libmp3lame -b:a 16k tone.mp3
    ffmpeg $F -ar 24000 -c:a mp2 -b:a 32k tone.mp2
    ffmpeg $F -ar 8000  -c:a libvorbis -q:a -1 tone.ogg
    ffmpeg $F -ar 8000  -c:a aac -b:a 16k tone.m4a
    ffmpeg $F -ar 8000  -c:a aac -b:a 16k -f adts tone.aac
    ffmpeg $F -ar 8000  -c:a pcm_s16le tone.caf
    ffmpeg $F -ar 8000  -c:a pcm_s16le tone.mkv
    ffmpeg $F -ar 8000  -c:a libvorbis -q:a -1 tone.webm
    ffmpeg $F -ar 8000  -c:a pcm_s16le -f aiff tone.aifc    # AIFF-C "sowt"

Made here with ffmpeg 9.0.2 (`-bitexact -map_metadata -1` added to `$F`):

    ffmpeg $F -ar 8000  -c:a pcm_s16le tone.mov
    ffmpeg $F -ar 8000  -c:a pcm_s16le -f matroska tone.mka
    ffmpeg $F -ar 48000 -c:a libopus -b:a 16k tone.opus
    ffmpeg $F -ar 48000 -c:a ac3 -b:a 64k tone.ac3
    ffmpeg $F -ar 48000 -c:a eac3 -b:a 64k tone.eac3

`silence.mp1` has no ffmpeg encoder. It is four hand-built MPEG-1 Layer I frames (mono, 32 kHz,
32 kbit/s, 48 bytes each: header `FF FF 18 C0`, all subband allocations zero), which decode to
silence.

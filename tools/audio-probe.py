#!/usr/bin/env python3
"""Measures whether the system's audio output consumes sound in real time.

Plays a 440 Hz tone for a few seconds and prints how long it took: if the output
works, N seconds of sound take about N seconds. It is used to tell whether a defect lies in
our way of writing or in the audio server (in WSL: WSLg's PulseAudio).

Usage:
  python3 -I tools/audio-probe.py pulse [block_ms] [queue_ms]   # libpulse-simple, like the runtime
  python3 -I tools/audio-probe.py sdl   [block_ms]              # SDL2, with a measurable queue
  python3 -I tools/audio-probe.py all                         # the useful combinations, one after another

block_ms: how much sound per write (the runtime's mixer uses 10). queue_ms: length of the
queue requested from PulseAudio (0 = the server's default).
Standard library only (ctypes). If the server does not respond, `pulse` can hang for
half a minute before giving up.
"""
import ctypes
import math
import struct
import sys
import time

RATE, SECONDS = 48000, 4


def tone(frames, start):
    return b"".join(struct.pack("<hh", *(2 * [int(6000 * math.sin((start + i) * 2 * math.pi * 440 / RATE))])) for i in range(frames))


def pulse(block_ms=10, queue_ms=0):
    pa = ctypes.CDLL("libpulse-simple.so.0")
    pa.pa_simple_new.restype = ctypes.c_void_p
    pa.pa_simple_new.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(ctypes.c_int)]
    pa.pa_simple_write.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_int)]
    pa.pa_simple_drain.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int)]
    pa.pa_simple_free.argtypes = [ctypes.c_void_p]
    spec = (ctypes.c_uint32 * 3)(3, RATE, 2)  # S16LE, 48 kHz, stereo
    attr = (ctypes.c_uint32 * 5)(0xFFFFFFFF, RATE * 4 // 1000 * queue_ms, 0xFFFFFFFF, 0xFFFFFFFF, 0xFFFFFFFF) if queue_ms else None
    err = ctypes.c_int(0)
    label = f"pulse, {block_ms} ms blocks, queue {queue_ms or 'default'}"
    t0 = time.time()
    h = pa.pa_simple_new(None, b"altair-audio-probe", 1, None, b"test", spec, None, attr, ctypes.byref(err))
    if not h:
        print(f"{label}: connection failed after {time.time() - t0:.1f} s (error {err.value}; 8 = the server does not respond)")
        return
    frames = RATE * block_ms // 1000
    t0, worst = time.time(), 0.0
    for i in range(SECONDS * 1000 // block_ms):
        # In real time, like the mixer: never faster than the clock.
        wait = t0 + i * block_ms / 1000 - time.time()
        if wait > 0:
            time.sleep(wait)
        t = time.time()
        pa.pa_simple_write(h, tone(frames, i * frames), frames * 4, ctypes.byref(err))
        worst = max(worst, time.time() - t)
    written = time.time() - t0
    pa.pa_simple_drain(h, ctypes.byref(err))
    print(f"{label}: {SECONDS} s of sound written in {written:.1f} s, drained in {time.time() - t0:.1f} s; slowest write {worst * 1000:.0f} ms")
    pa.pa_simple_free(h)


class Spec(ctypes.Structure):
    _fields_ = [("freq", ctypes.c_int), ("format", ctypes.c_uint16), ("channels", ctypes.c_uint8), ("silence", ctypes.c_uint8), ("samples", ctypes.c_uint16), ("padding", ctypes.c_uint16), ("size", ctypes.c_uint32), ("callback", ctypes.c_void_p), ("userdata", ctypes.c_void_p)]


def sdl(block_ms=10):
    s = ctypes.CDLL("libSDL2-2.0.so.0")
    s.SDL_GetError.restype = ctypes.c_char_p
    s.SDL_GetCurrentAudioDriver.restype = ctypes.c_char_p
    s.SDL_OpenAudioDevice.argtypes = [ctypes.c_char_p, ctypes.c_int, ctypes.POINTER(Spec), ctypes.POINTER(Spec), ctypes.c_int]
    s.SDL_QueueAudio.argtypes = [ctypes.c_uint32, ctypes.c_char_p, ctypes.c_uint32]
    if s.SDL_Init(0x10) != 0:  # SDL_INIT_AUDIO
        print(f"sdl: no audio device ({s.SDL_GetError().decode()})")
        return
    want, have = Spec(RATE, 0x8010, 2, 0, 480, 0, 0, None, None), Spec()  # AUDIO_S16LSB
    dev = s.SDL_OpenAudioDevice(None, 0, ctypes.byref(want), ctypes.byref(have), 0)
    if not dev:
        print(f"sdl: cannot open device ({s.SDL_GetError().decode()})")
        return
    s.SDL_PauseAudioDevice(dev, 0)
    frames = RATE * block_ms // 1000
    t0, most = time.time(), 0
    for i in range(SECONDS * 1000 // block_ms):
        wait = t0 + i * block_ms / 1000 - time.time()
        if wait > 0:
            time.sleep(wait)
        s.SDL_QueueAudio(dev, tone(frames, i * frames), frames * 4)
        most = max(most, s.SDL_GetQueuedAudioSize(dev))
    written = time.time() - t0
    while s.SDL_GetQueuedAudioSize(dev) > 0 and time.time() - t0 < 4 * SECONDS:
        time.sleep(0.01)
    left = s.SDL_GetQueuedAudioSize(dev)
    print(f"sdl ({s.SDL_GetCurrentAudioDriver().decode()}), {block_ms} ms blocks: {SECONDS} s of sound queued in {written:.1f} s, consumed in {time.time() - t0:.1f} s; maximum queue {most / (RATE * 4) * 1000:.0f} ms" + (f"; {left / (RATE * 4) * 1000:.0f} ms LEFT unconsumed" if left else ""))
    s.SDL_CloseAudioDevice(dev)
    s.SDL_Quit()


def main():
    args = sys.argv[1:]
    numbers = [int(a) for a in args[1:]]
    if not args:
        sys.exit(__doc__)
    if args[0] == "pulse":
        pulse(*numbers)
    elif args[0] == "sdl":
        sdl(*numbers)
    elif args[0] == "all":
        sdl(10)
        for block, queue in [(10, 0), (10, 200), (10, 60), (50, 0)]:
            pulse(block, queue)
    else:
        sys.exit(__doc__)


main()

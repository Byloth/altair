#!/usr/bin/env python3
"""Misura se l'uscita audio del sistema consuma il suono in tempo reale.

Suona un tono a 440 Hz per qualche secondo e stampa quanto tempo ci è voluto: se l'uscita
funziona, N secondi di suono richiedono circa N secondi. Serve a capire se un difetto sta nel
nostro modo di scrivere o nel server audio (in WSL: PulseAudio di WSLg).

Uso:
  python3 -I tools/audio-probe.py pulse [blocco_ms] [coda_ms]   # libpulse-simple, come il runtime
  python3 -I tools/audio-probe.py sdl   [blocco_ms]             # SDL2, con coda misurabile
  python3 -I tools/audio-probe.py tutto                         # le combinazioni utili, in fila

blocco_ms: quanto suono per scrittura (il mixer del runtime usa 10). coda_ms: lunghezza della
coda chiesta a PulseAudio (0 = quella predefinita del server).
Solo libreria standard (ctypes). Se il server non risponde, `pulse` può restare fermo
mezzo minuto prima di arrendersi.
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
    label = f"pulse, blocchi da {block_ms} ms, coda {queue_ms or 'predefinita'}"
    t0 = time.time()
    h = pa.pa_simple_new(None, b"altair-audio-probe", 1, None, b"prova", spec, None, attr, ctypes.byref(err))
    if not h:
        print(f"{label}: connessione fallita dopo {time.time() - t0:.1f} s (errore {err.value}; 8 = il server non risponde)")
        return
    frames = RATE * block_ms // 1000
    t0, worst = time.time(), 0.0
    for i in range(SECONDS * 1000 // block_ms):
        # A tempo reale, come il mixer: mai più veloce dell'orologio.
        wait = t0 + i * block_ms / 1000 - time.time()
        if wait > 0:
            time.sleep(wait)
        t = time.time()
        pa.pa_simple_write(h, tone(frames, i * frames), frames * 4, ctypes.byref(err))
        worst = max(worst, time.time() - t)
    written = time.time() - t0
    pa.pa_simple_drain(h, ctypes.byref(err))
    print(f"{label}: {SECONDS} s di suono scritti in {written:.1f} s, svuotati in {time.time() - t0:.1f} s; scrittura più lenta {worst * 1000:.0f} ms")
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
        print(f"sdl: nessun dispositivo audio ({s.SDL_GetError().decode()})")
        return
    want, have = Spec(RATE, 0x8010, 2, 0, 480, 0, 0, None, None), Spec()  # AUDIO_S16LSB
    dev = s.SDL_OpenAudioDevice(None, 0, ctypes.byref(want), ctypes.byref(have), 0)
    if not dev:
        print(f"sdl: dispositivo non apribile ({s.SDL_GetError().decode()})")
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
    print(f"sdl ({s.SDL_GetCurrentAudioDriver().decode()}), blocchi da {block_ms} ms: {SECONDS} s di suono accodati in {written:.1f} s, consumati in {time.time() - t0:.1f} s; coda massima {most / (RATE * 4) * 1000:.0f} ms" + (f"; RIMASTI {left / (RATE * 4) * 1000:.0f} ms non consumati" if left else ""))
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
    elif args[0] == "tutto":
        sdl(10)
        for block, queue in [(10, 0), (10, 200), (10, 60), (50, 0)]:
            pulse(block, queue)
    else:
        sys.exit(__doc__)


main()

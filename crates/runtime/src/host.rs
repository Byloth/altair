//! Ciò che cambia tra i sistemi su cui gira il runtime: caricare la libreria con il codice
//! tradotto, proteggere la memoria guest e accorgersi degli accessi sbagliati, restituire
//! memoria, uscire subito, far uscire il suono. Tutto il resto del runtime è uguale su Linux
//! e su Windows.

use std::ffi::c_void;

/// Frequenza dell'uscita audio: 48 kHz, stereo, 16 bit.
pub const AUDIO_RATE: u32 = 48_000;

/// Chiamata quando il codice tocca memoria che non può toccare; l'indirizzo è quello host.
/// Stampa ciò che sa e dice se l'accesso riguardava la memoria guest: in quel caso il processo
/// esce. Su Windows un accesso non valido altrove può essere affare di qualcun altro (un driver
/// che lo gestisce da sé) e lo si lascia proseguire per la sua strada.
pub type FaultReport = fn(addr: usize) -> bool;

#[cfg(unix)]
mod imp {
    use super::*;
    use std::ffi::{CStr, CString};

    pub struct Library(*mut c_void);

    impl Library {
        pub fn open(path: &str) -> Result<Library, String> {
            let c = CString::new(path).map_err(|e| e.to_string())?;
            // SAFETY: carica una libreria condivisa; il costruttore non ha altri effetti.
            let lib = unsafe { libc::dlopen(c.as_ptr(), libc::RTLD_NOW) };
            if lib.is_null() {
                return Err(format!("dlopen: {}", unsafe { CStr::from_ptr(libc::dlerror()) }.to_string_lossy()));
            }
            Ok(Library(lib))
        }
        pub fn symbol(&self, name: &str) -> *mut c_void {
            let c = CString::new(name).unwrap();
            unsafe { libc::dlsym(self.0, c.as_ptr()) }
        }
    }

    static REPORT: std::sync::OnceLock<FaultReport> = std::sync::OnceLock::new();

    extern "C" fn on_fault(_: libc::c_int, info: *mut libc::siginfo_t, _: *mut c_void) {
        // Non è codice "sicuro per i segnali" (stampa, può prendere un lock), ma il processo
        // sta morendo comunque e quello che stampa è l'unica traccia utile.
        let addr = unsafe { (*info).si_addr() } as usize;
        if let Some(report) = REPORT.get() {
            report(addr);
        }
        unsafe { libc::_exit(3) }
    }

    pub fn guard(base: *mut u8, null_page: bool, report: FaultReport) {
        let _ = REPORT.set(report);
        unsafe {
            if null_page {
                libc::mprotect(base.cast(), 0x1_0000, libc::PROT_NONE);
            }
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_fault as *const () as usize;
            action.sa_flags = libc::SA_SIGINFO;
            libc::sigaction(libc::SIGSEGV, &action, std::ptr::null_mut());
        }
    }

    pub fn discard(ptr: *mut u8, len: usize) {
        // SAFETY: la zona appartiene allo spazio guest; MADV_DONTNEED la riporta a zero.
        unsafe { libc::madvise(ptr.cast(), len, libc::MADV_DONTNEED) };
    }

    pub fn exit_now(code: i32) -> ! {
        unsafe { libc::_exit(code) }
    }

    /// Su Linux i timer sono già fini.
    pub fn fine_timers() {}

    /// PulseAudio attraverso `libpulse-simple`, caricata a tempo di esecuzione.
    pub struct AudioOut {
        handle: *mut c_void,
        write: unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut i32) -> i32,
    }

    impl AudioOut {
        pub fn open() -> Option<AudioOut> {
            unsafe {
                let lib = libc::dlopen(c"libpulse-simple.so.0".as_ptr(), libc::RTLD_NOW);
                if lib.is_null() {
                    eprintln!("[audio] libpulse-simple non trovata: mixer muto");
                    return None;
                }
                let new = libc::dlsym(lib, c"pa_simple_new".as_ptr());
                let write = libc::dlsym(lib, c"pa_simple_write".as_ptr());
                if new.is_null() || write.is_null() {
                    return None;
                }
                type New = unsafe extern "C" fn(*const i8, *const i8, i32, *const i8, *const i8, *const u32, *const u8, *const u32, *mut i32) -> *mut c_void;
                let new: New = std::mem::transmute(new);
                // pa_sample_spec { format, rate, channels }: S16LE (3), 48 kHz, stereo.
                let spec = [3u32, AUDIO_RATE, 2];
                // pa_buffer_attr { maxlength, tlength, prebuf, minreq, fragsize }. La coda è lunga
                // (`ALTAIR_AUDIO_MS`, 200 ms): con 60 ms il sink di WSLg rallenta fino a un quinto
                // del tempo reale, per tutti i programmi.
                let ms = std::env::var("ALTAIR_AUDIO_MS").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(200).clamp(20, 2000);
                let attr = [u32::MAX, AUDIO_RATE * 4 / 1000 * ms, u32::MAX, u32::MAX, u32::MAX];
                let mut err = 0;
                let handle = new(std::ptr::null(), c"altair".as_ptr(), 1, std::ptr::null(), c"Assassin's Creed".as_ptr(), spec.as_ptr(), std::ptr::null(), attr.as_ptr(), &mut err);
                if handle.is_null() {
                    eprintln!("[audio] PulseAudio non raggiungibile (errore {err}): mixer muto");
                    return None;
                }
                eprintln!("[audio] uscita su PulseAudio, 48 kHz stereo");
                Some(AudioOut { handle, write: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut i32) -> i32>(write) })
            }
        }

        /// Scrive un blocco di campioni; aspetta se la coda del server è piena.
        pub fn write(&mut self, pcm: &[u8]) {
            let mut err = 0;
            unsafe { (self.write)(self.handle, pcm.as_ptr(), pcm.len(), &mut err) };
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        fn GetLastError() -> u32;
        fn VirtualProtect(addr: *mut c_void, size: usize, protect: u32, old: *mut u32) -> i32;
        fn AddVectoredExceptionHandler(first: u32, handler: unsafe extern "system" fn(*mut ExceptionPointers) -> i32) -> *mut c_void;
        fn ExitProcess(code: u32) -> !;
        fn Sleep(ms: u32);
    }

    pub struct Library(*mut c_void);

    impl Library {
        pub fn open(path: &str) -> Result<Library, String> {
            // Percorso assoluto: così la ricerca delle DLL di Windows non c'entra.
            let full = std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))?;
            let wide: Vec<u16> = full.as_os_str().encode_wide().chain([0]).collect();
            let lib = unsafe { LoadLibraryW(wide.as_ptr()) };
            if lib.is_null() {
                return Err(format!("LoadLibrary {}: errore {}", full.display(), unsafe { GetLastError() }));
            }
            Ok(Library(lib))
        }
        pub fn symbol(&self, name: &str) -> *mut c_void {
            let c = std::ffi::CString::new(name).unwrap();
            unsafe { GetProcAddress(self.0, c.as_ptr().cast()) }
        }
    }

    /// EXCEPTION_RECORD: codice, flag, record collegato, indirizzo del codice, numero di
    /// parametri, parametri. Per una violazione di accesso il secondo parametro è l'indirizzo.
    #[repr(C)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        next: *mut ExceptionRecord,
        address: *mut c_void,
        count: u32,
        info: [usize; 15],
    }
    #[repr(C)]
    struct ExceptionPointers {
        record: *mut ExceptionRecord,
        context: *mut c_void,
    }

    static REPORT: std::sync::OnceLock<FaultReport> = std::sync::OnceLock::new();

    unsafe extern "system" fn on_fault(info: *mut ExceptionPointers) -> i32 {
        const ACCESS_VIOLATION: u32 = 0xc000_0005;
        let record = unsafe { &*(*info).record };
        if record.code != ACCESS_VIOLATION {
            return 0; // EXCEPTION_CONTINUE_SEARCH: non è affar nostro
        }
        match REPORT.get() {
            Some(report) if report(record.info[1]) => unsafe { ExitProcess(3) },
            _ => 0,
        }
    }

    pub fn guard(base: *mut u8, null_page: bool, report: FaultReport) {
        let _ = REPORT.set(report);
        unsafe {
            if null_page {
                let mut old = 0u32;
                VirtualProtect(base.cast(), 0x1_0000, 1, &mut old); // PAGE_NOACCESS
            }
            AddVectoredExceptionHandler(1, on_fault);
        }
    }

    pub fn discard(ptr: *mut u8, len: usize) {
        // Windows non ha un equivalente economico di MADV_DONTNEED che riazzeri: si azzera a mano.
        unsafe { std::ptr::write_bytes(ptr, 0, len) };
    }

    pub fn exit_now(code: i32) -> ! {
        unsafe { ExitProcess(code as u32) }
    }

    #[link(name = "winmm")]
    extern "system" {
        fn timeBeginPeriod(ms: u32) -> u32;
    }

    /// Porta a 1 ms il passo dei timer di sistema, che altrimenti è di 15,6 ms: le attese
    /// con scadenza dei thread del gioco (quello dell'audio riempie i buffer a segmenti di
    /// 60 ms) arriverebbero in ritardo. È ciò che fa ogni gioco su Windows.
    pub fn fine_timers() {
        unsafe { timeBeginPeriod(1) };
    }

    // --- uscita audio: l'API waveOut, la più semplice che Windows offra -----------------------

    #[repr(C, packed)]
    struct WaveFormat {
        tag: u16,
        channels: u16,
        rate: u32,
        bytes_per_sec: u32,
        block: u16,
        bits: u16,
        extra: u16,
    }
    #[repr(C)]
    struct WaveHeader {
        data: *mut u8,
        length: u32,
        recorded: u32,
        user: usize,
        flags: u32,
        loops: u32,
        next: *mut WaveHeader,
        reserved: usize,
    }
    const HEADER_DONE: u32 = 1;
    const HEADER_PREPARED: u32 = 2;

    #[link(name = "winmm")]
    extern "system" {
        fn waveOutOpen(handle: *mut *mut c_void, device: u32, format: *const WaveFormat, callback: usize, instance: usize, flags: u32) -> u32;
        fn waveOutPrepareHeader(handle: *mut c_void, header: *mut WaveHeader, size: u32) -> u32;
        fn waveOutUnprepareHeader(handle: *mut c_void, header: *mut WaveHeader, size: u32) -> u32;
        fn waveOutWrite(handle: *mut c_void, header: *mut WaveHeader, size: u32) -> u32;
    }

    /// Una manciata di blocchi che girano: si scrive in uno libero, lo si consegna, e quando la
    /// scheda l'ha suonato torna libero. Se sono tutti in coda si aspetta: è il freno.
    pub struct AudioOut {
        handle: *mut c_void,
        blocks: Vec<(Box<WaveHeader>, Vec<u8>)>,
    }

    impl AudioOut {
        pub fn open() -> Option<AudioOut> {
            let format = WaveFormat { tag: 1, channels: 2, rate: AUDIO_RATE, bytes_per_sec: AUDIO_RATE * 4, block: 4, bits: 16, extra: 0 };
            let mut handle = std::ptr::null_mut();
            // WAVE_MAPPER: il dispositivo predefinito.
            let err = unsafe { waveOutOpen(&mut handle, u32::MAX, &format, 0, 0, 0) };
            if err != 0 {
                eprintln!("[audio] nessun dispositivo di uscita (waveOutOpen: errore {err}): mixer muto");
                return None;
            }
            // Venti blocchi da 10 ms: fino a 200 ms di coda.
            let blocks = (0..20).map(|_| (Box::new(WaveHeader { data: std::ptr::null_mut(), length: 0, recorded: 0, user: 0, flags: 0, loops: 0, next: std::ptr::null_mut(), reserved: 0 }), Vec::new())).collect();
            eprintln!("[audio] uscita su waveOut, 48 kHz stereo");
            Some(AudioOut { handle, blocks })
        }

        pub fn write(&mut self, pcm: &[u8]) {
            let size = std::mem::size_of::<WaveHeader>() as u32;
            loop {
                for (header, data) in self.blocks.iter_mut() {
                    let flags = unsafe { std::ptr::read_volatile(&header.flags) };
                    let free = flags & HEADER_PREPARED == 0 || flags & HEADER_DONE != 0;
                    if !free {
                        continue;
                    }
                    unsafe {
                        if flags & HEADER_PREPARED != 0 {
                            waveOutUnprepareHeader(self.handle, &mut **header, size);
                        }
                        data.clear();
                        data.extend_from_slice(pcm);
                        header.data = data.as_mut_ptr();
                        header.length = data.len() as u32;
                        header.flags = 0;
                        waveOutPrepareHeader(self.handle, &mut **header, size);
                        waveOutWrite(self.handle, &mut **header, size);
                    }
                    return;
                }
                unsafe { Sleep(1) };
            }
        }
    }
}

pub use imp::{discard, exit_now, fine_timers, guard, AudioOut, Library};

// I puntatori dentro sono handle del sistema, usati da un thread solo alla volta.
unsafe impl Send for AudioOut {}

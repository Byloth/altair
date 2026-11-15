//! What changes between the systems the runtime runs on: loading the library with the
//! translated code, protecting guest memory and noticing bad accesses, giving memory back,
//! exiting immediately, getting sound out. The rest of the runtime is the same on Linux
//! and on Windows.

use std::ffi::c_void;

/// Audio output rate: 48 kHz, stereo, 16 bit.
pub const AUDIO_RATE: u32 = 48_000;

/// Called when the code touches memory it must not touch; the address is the host one.
/// It prints what it knows and says whether the access concerned guest memory: in that case the
/// process exits. On Windows an invalid access elsewhere may be someone else's business (a driver
/// that handles it by itself) and it is left to go its own way.
pub type FaultReport = fn(addr: usize) -> bool;

#[cfg(unix)]
mod imp {
    use super::*;
    use std::ffi::{CStr, CString};

    pub struct Library(*mut c_void);

    impl Library {
        pub fn open(path: &str) -> Result<Library, String> {
            let c = CString::new(path).map_err(|e| e.to_string())?;
            // SAFETY: loads a shared library; the constructor has no other effects.
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

    extern "C" fn on_fault(_: libc::c_int, info: *mut libc::siginfo_t, context: *mut c_void) {
        // This is not "signal-safe" code (it prints, it may take a lock), but the process
        // is dying anyway and what it prints is the only useful trace.
        let addr = unsafe { (*info).si_addr() } as usize;
        // Where it was executing: for translated code the symbol is the guest function (f_XXXXXXXX).
        #[cfg(target_arch = "x86_64")]
        unsafe {
            let rip = (*(context as *mut libc::ucontext_t)).uc_mcontext.gregs[libc::REG_RIP as usize] as usize;
            let mut sym: libc::Dl_info = std::mem::zeroed();
            if libc::dladdr(rip as *const c_void, &mut sym) != 0 && !sym.dli_sname.is_null() {
                eprintln!("\ninvalid access inside {} (+{:#x})", CStr::from_ptr(sym.dli_sname).to_string_lossy(), rip - sym.dli_saddr as usize);
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        let _ = context;
        // Investigation (`ALTAIR_TRAP_CONTINUE=1`): the page becomes writable again and
        // execution continues. With a page made read-only on purpose, the log thus lists, one
        // after the other, the functions that write to it.
        if addr >= 0x1_0000 && std::env::var_os("ALTAIR_TRAP_CONTINUE").is_some() {
            unsafe { libc::mprotect((addr & !0xfff) as *mut c_void, 0x1000, libc::PROT_READ | libc::PROT_WRITE) };
            return;
        }
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

    /// Makes the pages containing the region read-only: the next write ends up in the
    /// invalid-access handler, which says which function made it. For investigations only.
    pub fn write_trap(ptr: *mut u8, len: usize) {
        let start = ptr as usize & !0xfff;
        let end = (ptr as usize + len + 0xfff) & !0xfff;
        unsafe { libc::mprotect(start as *mut c_void, end - start, libc::PROT_READ) };
    }

    pub fn discard(ptr: *mut u8, len: usize) {
        // SAFETY: the region belongs to the guest space; MADV_DONTNEED resets it to zero.
        unsafe { libc::madvise(ptr.cast(), len, libc::MADV_DONTNEED) };
    }

    pub fn exit_now(code: i32) -> ! {
        unsafe { libc::_exit(code) }
    }

    /// On Linux the timers are already fine-grained.
    pub fn fine_timers() {}

    /// PulseAudio through `libpulse-simple`, loaded at run time.
    pub struct AudioOut {
        handle: *mut c_void,
        write: unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut i32) -> i32,
    }

    impl AudioOut {
        pub fn open() -> Option<AudioOut> {
            unsafe {
                let lib = libc::dlopen(c"libpulse-simple.so.0".as_ptr(), libc::RTLD_NOW);
                if lib.is_null() {
                    eprintln!("[audio] libpulse-simple not found: mixer muted");
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
                // pa_buffer_attr { maxlength, tlength, prebuf, minreq, fragsize }. The queue is long
                // (`ALTAIR_AUDIO_MS`, 200 ms): with 60 ms the WSLg sink slows down to a fifth
                // of real time, for every program.
                let ms = std::env::var("ALTAIR_AUDIO_MS").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(200).clamp(20, 2000);
                let attr = [u32::MAX, AUDIO_RATE * 4 / 1000 * ms, u32::MAX, u32::MAX, u32::MAX];
                let mut err = 0;
                let handle = new(std::ptr::null(), c"altair".as_ptr(), 1, std::ptr::null(), c"Assassin's Creed".as_ptr(), spec.as_ptr(), std::ptr::null(), attr.as_ptr(), &mut err);
                if handle.is_null() {
                    eprintln!("[audio] PulseAudio unreachable (error {err}): mixer muted");
                    return None;
                }
                eprintln!("[audio] output on PulseAudio, 48 kHz stereo");
                Some(AudioOut { handle, write: std::mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut c_void, *const u8, usize, *mut i32) -> i32>(write) })
            }
        }

        /// Writes a block of samples; waits if the server's queue is full.
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
            // Absolute path: this way the Windows DLL search does not come into play.
            let full = std::fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))?;
            let wide: Vec<u16> = full.as_os_str().encode_wide().chain([0]).collect();
            let lib = unsafe { LoadLibraryW(wide.as_ptr()) };
            if lib.is_null() {
                return Err(format!("LoadLibrary {}: error {}", full.display(), unsafe { GetLastError() }));
            }
            Ok(Library(lib))
        }
        pub fn symbol(&self, name: &str) -> *mut c_void {
            let c = std::ffi::CString::new(name).unwrap();
            unsafe { GetProcAddress(self.0, c.as_ptr().cast()) }
        }
    }

    /// EXCEPTION_RECORD: code, flags, linked record, code address, number of
    /// parameters, parameters. For an access violation the second parameter is the address.
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
            return 0; // EXCEPTION_CONTINUE_SEARCH: not our business
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
        // Windows has no cheap equivalent of MADV_DONTNEED that zeroes again: it is zeroed by hand.
        unsafe { std::ptr::write_bytes(ptr, 0, len) };
    }

    pub fn exit_now(code: i32) -> ! {
        unsafe { ExitProcess(code as u32) }
    }

    #[link(name = "winmm")]
    extern "system" {
        fn timeBeginPeriod(ms: u32) -> u32;
    }

    /// Brings the system timer step down to 1 ms, which is otherwise 15.6 ms: the timed
    /// waits of the game's threads (the audio one fills the buffers in 60 ms segments)
    /// would arrive late. It is what every game does on Windows.
    pub fn fine_timers() {
        unsafe { timeBeginPeriod(1) };
    }

    // --- audio output: the waveOut API, the simplest one Windows offers ------------------------

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

    /// A handful of blocks in rotation: one writes into a free one, hands it over, and when the
    /// card has played it, it becomes free again. If they are all queued, one waits: that is the brake.
    pub struct AudioOut {
        handle: *mut c_void,
        blocks: Vec<(Box<WaveHeader>, Vec<u8>)>,
    }

    impl AudioOut {
        pub fn open() -> Option<AudioOut> {
            let format = WaveFormat { tag: 1, channels: 2, rate: AUDIO_RATE, bytes_per_sec: AUDIO_RATE * 4, block: 4, bits: 16, extra: 0 };
            let mut handle = std::ptr::null_mut();
            // WAVE_MAPPER: the default device.
            let err = unsafe { waveOutOpen(&mut handle, u32::MAX, &format, 0, 0, 0) };
            if err != 0 {
                eprintln!("[audio] no output device (waveOutOpen: error {err}): mixer muted");
                return None;
            }
            // Twenty 10 ms blocks: up to 200 ms of queue.
            let blocks = (0..20).map(|_| (Box::new(WaveHeader { data: std::ptr::null_mut(), length: 0, recorded: 0, user: 0, flags: 0, loops: 0, next: std::ptr::null_mut(), reserved: 0 }), Vec::new())).collect();
            eprintln!("[audio] output on waveOut, 48 kHz stereo");
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
#[cfg(unix)]
pub use imp::write_trap;

// The pointers inside are system handles, used by one thread at a time.
unsafe impl Send for AudioOut {}

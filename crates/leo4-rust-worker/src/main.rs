//! `leo4-rust-worker` — Phase 9-3 worker harness.
//!
//! The dispatcher (`libleo4_rust_bridge.a`, Phase 9-4) spawns
//! one instance of this binary per cdylib (or per call when
//! the caller marks an export `isolated`). The worker:
//!
//! 1. Opens the cdylib via `dlopen` / `LoadLibrary`.
//! 2. Walks the cdylib's `EXPORTS` slice through
//!    `leo4_rust_describe_exports`, computes the same FNV-1a-64
//!    `schema_hash` that `leo4-rust-emit` writes to the handshake
//!    file, and sends a handshake frame back to the dispatcher.
//! 3. Enters a request loop: read a request frame, `dlsym` the
//!    requested mangled symbol (cached after first lookup),
//!    invoke it under `catch_unwind`, and write a response
//!    frame.
//! 4. Exits cleanly when the dispatcher sends a magic=0 request
//!    (graceful shutdown signal).
//!
//! Wire format: SPEC/reverse-direction.md §5.
//! Worker lifecycle: §4.1.

#![allow(clippy::missing_errors_doc)]

use std::{
    collections::HashMap,
    io::{Read, Write},
    path::PathBuf,
    process,
};

use clap::Parser;
use leo4_abi::rust_exports::ExportEntry;

// ─── Wire format constants ─────────────────────────────────────────

const FRAME_MAGIC: u32 = 0x4C45; // 'LE'
const ABI_VERSION: u32 = 1;
const SCHEMA_HASH_LEN: u32 = 13;

// ─── Wrapper FFI signature ─────────────────────────────────────────
//
// Every `#[leo4::export]`-generated wrapper symbol has this shape;
// the worker resolves the symbol by mangled name and casts the
// resulting pointer to this type.

type WrapperFn = unsafe extern "C" fn(
    args_ptr: *const u8,
    args_len: usize,
    ret_ptr: *mut u8,
    ret_cap: usize,
    ret_len: *mut usize,
) -> i32;

// ─── Error codes (subset surfaced by the worker) ───────────────────
//
// Full table in SPEC/reverse-direction.md §10 and
// SPEC/canonical-abi.md §13.

const LEO4_OK: i32 = 0;
const LEO4_ERR_DECODE: i32 = 0x0000_0001;
const LEO4_ERR_BUFFER_TOO_SMALL: i32 = 0x0000_0007;
#[allow(dead_code)]
const LEO4_ERR_RUST_PANIC: i32 = 0x0002_0001;
const LEO4_ERR_RUST_DLSYM_FAILED: i32 = 0x0002_0005;

// ─── CLI ───────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(name = "leo4-rust-worker", version, about = "Phase 9 reverse-direction worker harness")]
struct Cli {
    /// Path to the user cdylib.
    #[arg(long)]
    cdylib: PathBuf,

    /// IPC channel for the request loop. POSIX: a numeric fd
    /// inherited from the dispatcher via `posix_spawn`. Windows
    /// uses `--ipc-pipe` instead (not exercised in v0).
    #[cfg(unix)]
    #[arg(long)]
    ipc_fd: Option<i32>,

    /// IPC channel for the request loop on Windows: a named pipe
    /// path (`\\.\pipe\leo4_rust_*`) created by the dispatcher.
    #[cfg(windows)]
    #[arg(long)]
    ipc_pipe: Option<String>,

    /// Initial response buffer capacity, in bytes. The worker
    /// grows on demand when a wrapper returns
    /// `LEO4_ERR_BUFFER_TOO_SMALL`. Default 64 KiB.
    #[arg(long, default_value_t = 65_536)]
    ret_cap: usize,
}

// ─── main ──────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("leo4-rust-worker: {e}");
        process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    // SAFETY: dlopening an arbitrary cdylib is the worker's whole
    // purpose; the dispatcher passed the path.
    let lib = unsafe {
        libloading::Library::new(&cli.cdylib)
            .map_err(|e| format!("dlopen {:?}: {e}", cli.cdylib))?
    };

    let entries = load_entries(&lib)?;
    let schema_hash = compute_schema_hash_for(&entries);

    let mut channel = open_ipc_channel(&cli)?;
    send_handshake(&mut channel, &schema_hash)?;

    let mut cache: HashMap<String, WrapperFn> = HashMap::new();
    let mut ret_buf: Vec<u8> = vec![0u8; cli.ret_cap];

    loop {
        let req = match read_request_frame(&mut channel)? {
            Some(r) => r,
            None => break, // graceful shutdown
        };

        let (status, response_bytes, detail) =
            handle_request(&lib, &mut cache, &mut ret_buf, &req);

        write_response_frame(&mut channel, status, &response_bytes, &detail)?;
    }
    Ok(())
}

// ─── cdylib introspection ──────────────────────────────────────────

#[derive(Debug, Clone)]
struct EntryView {
    logical_name: String,
    mangled: String,
    param_types: Vec<String>,
    ret_type: String,
    /// Reserved — dispatcher (not worker) honours `isolated`; we
    /// keep the field so future per-entry policy can be enforced
    /// worker-side (e.g. recycle on first isolated call).
    #[allow(dead_code)]
    isolated: bool,
    /// Reserved — Phase 9.X will gate per-entry behaviour on
    /// the `abi_version` stored here vs the worker's
    /// `ABI_VERSION`.
    #[allow(dead_code)]
    abi_version: u32,
}

fn load_entries(lib: &libloading::Library) -> Result<Vec<EntryView>, String> {
    type Describe = unsafe extern "C" fn(*mut *const ExportEntry, *mut usize) -> i32;
    // SAFETY: symbol type matches the leo4-abi extern signature.
    let sym: libloading::Symbol<'_, Describe> = unsafe {
        lib.get(b"leo4_rust_describe_exports\0").map_err(|e| {
            format!(
                "cdylib missing `leo4_rust_describe_exports`: {e}. \
                 Did the cdylib enable the `leo4` `rust-exports` feature?"
            )
        })?
    };

    let mut ptr: *const ExportEntry = std::ptr::null();
    let mut len: usize = 0;
    // SAFETY: out-pointers are valid; symbol signature matches.
    let rc = unsafe { sym(&raw mut ptr, &raw mut len) };
    if rc != 0 {
        return Err(format!("leo4_rust_describe_exports rc={rc}"));
    }
    if len > 0 && ptr.is_null() {
        return Err("describe returned non-zero length with null pointer".into());
    }

    let mut out = Vec::with_capacity(len);
    // SAFETY: the slice is valid for the cdylib's lifetime.
    let slice: &[ExportEntry] = unsafe { std::slice::from_raw_parts(ptr, len) };
    for e in slice {
        out.push(EntryView {
            logical_name: e.logical_name.to_owned(),
            mangled: e.mangled.to_owned(),
            param_types: e.param_types.iter().map(|s| (*s).to_owned()).collect(),
            ret_type: e.ret_type.to_owned(),
            isolated: e.isolated,
            abi_version: e.abi_version,
        });
    }
    Ok(out)
}

// ─── schema_hash (same algorithm as leo4-rust-emit) ────────────────

fn compute_schema_hash_for(entries: &[EntryView]) -> String {
    // Render the collapsed canonical form, exports sorted by
    // mangled name. The package / interface tokens are not known
    // to the worker (they live in the handshake JSON the emit CLI
    // wrote), so we render with placeholder names that the
    // dispatcher's hash check must ALSO use. Since the dispatcher
    // doesn't itself recompute the hash — it reads it out of the
    // handshake JSON — we instead replicate the *exact* canonical
    // text the emit CLI used. The pkg/iface for the worker's
    // recomputation are taken from environment overrides
    // (LEO4_RUST_HANDSHAKE_PKG / _IFACE) when present; otherwise
    // we fall back to "rust" / "Rust" sentinels that the emit CLI
    // also uses when --pkg / --iface are unspecified.
    let pkg = std::env::var("LEO4_RUST_HANDSHAKE_PKG").unwrap_or_else(|_| "rust".into());
    let iface = std::env::var("LEO4_RUST_HANDSHAKE_IFACE").unwrap_or_else(|_| "Rust".into());

    let mut sorted: Vec<&EntryView> = entries.iter().collect();
    sorted.sort_by(|a, b| a.mangled.cmp(&b.mangled));

    let mut s = String::new();
    s.push_str(&format!("package {pkg}; interface {iface} {{ "));
    for e in &sorted {
        s.push_str(&format!("func {}(", e.logical_name));
        for (i, p) in e.param_types.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&format!("_{i}: {}", surface_form(p)));
        }
        s.push(')');
        if !e.ret_type.is_empty() {
            s.push_str(&format!(" -> {}", surface_form(&e.ret_type)));
        }
        s.push(';');
        s.push(' ');
    }
    s.push('}');

    fnv1a64_base32lc(&s)
}

fn surface_form(mangle: &str) -> String {
    // Minimal subset mirror of leo4-rust-emit's parse_mangle ->
    // idl_to_surface. Worker only needs to render enough to make
    // the hash bytes match; the emit CLI's choice of surface
    // form for unknown tokens is "raw mangle", so we do the same.
    match mangle {
        "u8" => "u8".into(),
        "u16" => "u16".into(),
        "u32" => "u32".into(),
        "u64" => "u64".into(),
        "i8" => "i8".into(),
        "i16" => "i16".into(),
        "i32" => "i32".into(),
        "i64" => "i64".into(),
        "f32" => "f32".into(),
        "f64" => "f64".into(),
        "b" => "bool".into(),
        "c" => "char".into(),
        "str" => "string".into(),
        "bI" => "bigint".into(),
        "bN" => "bignat".into(),
        other => {
            if let Some(rest) =
                other.strip_prefix("L_").and_then(|r| r.strip_suffix("_l"))
            {
                format!("list<{}>", surface_form(rest))
            } else if let Some(rest) =
                other.strip_prefix("O_").and_then(|r| r.strip_suffix("_o"))
            {
                format!("option<{}>", surface_form(rest))
            } else if let Some(rest) =
                other.strip_prefix("S_").and_then(|r| r.strip_suffix("_s"))
            {
                rest.replace('_', ".")
            } else {
                other.to_string()
            }
        }
    }
}

fn fnv1a64_base32lc(s: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for &b in s.as_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(PRIME);
    }
    base32lc(&h.to_be_bytes())
}

const BASE32_LC: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

fn base32lc(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(5) * 8);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &b in input {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(BASE32_LC[((buf >> bits) & 0x1f) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(BASE32_LC[((buf << (5 - bits)) & 0x1f) as usize]));
    }
    out
}

// ─── IPC channel (POSIX socketpair / Windows named pipe) ───────────
//
// v9-3 implements the POSIX path; Windows lands with 9-4c.

#[cfg(unix)]
fn open_ipc_channel(cli: &Cli) -> Result<Box<dyn ReadWrite>, String> {
    use std::os::unix::io::FromRawFd;
    use std::os::unix::net::UnixStream;

    let Some(fd) = cli.ipc_fd else {
        return Err("--ipc-fd required on POSIX targets".into());
    };
    if fd < 0 {
        return Err(format!("--ipc-fd must be non-negative, got {fd}"));
    }
    // SAFETY: the dispatcher passed an inherited fd; we own it.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    Ok(Box::new(stream))
}

#[cfg(windows)]
fn open_ipc_channel(cli: &Cli) -> Result<Box<dyn ReadWrite>, String> {
    let Some(pipe) = cli.ipc_pipe.as_ref() else {
        return Err("--ipc-pipe required on Windows targets".into());
    };
    open_windows_pipe(pipe).map(|f| Box::new(f) as Box<dyn ReadWrite>)
}

/// Open a duplex named pipe at `pipe_path` (the path the
/// dispatcher created via `CreateNamedPipeA`,
/// `\\.\pipe\leo4_rust_<pid>_<nonce>` per
/// `shim/leo4_rust_bridge.c` § "Windows backend
/// (Phase 9-4c)").
///
/// `std::fs::OpenOptions::open` on Windows calls
/// `CreateFileW` under the hood, which is the
/// client-side counterpart to `CreateNamedPipeA` /
/// `ConnectNamedPipe`. Read+write maps to
/// `GENERIC_READ | GENERIC_WRITE`, matching the
/// dispatcher's `PIPE_ACCESS_DUPLEX`. Byte-stream mode
/// (`PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT`
/// on the server side) lines up with `File`'s
/// blocking `Read` / `Write` impls.
///
/// **Retry loop**: the dispatcher's normal flow is
/// `CreateNamedPipeA` → `CreateProcessA(worker)` →
/// `ConnectNamedPipe` (blocks). The worker's
/// `open_ipc_channel` typically runs *before* the
/// dispatcher reaches `ConnectNamedPipe`, but a
/// narrow race exists where the worker process starts
/// before the named pipe is fully registered. Retry a
/// bounded number of times with linear backoff (10ms,
/// 20ms, …, 100ms) on `NotFound` / `ConnectionRefused`
/// — those are the two error kinds Windows surfaces
/// when the pipe doesn't yet exist or already has its
/// instance count saturated.
#[cfg(windows)]
fn open_windows_pipe(pipe_path: &str) -> Result<std::fs::File, String> {
    use std::fs::OpenOptions;
    use std::io::ErrorKind;
    use std::thread;
    use std::time::Duration;

    let mut last_err: Option<std::io::Error> = None;
    for attempt in 0..10u32 {
        match OpenOptions::new().read(true).write(true).open(pipe_path) {
            Ok(f) => return Ok(f),
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::NotFound | ErrorKind::ConnectionRefused
                ) =>
            {
                last_err = Some(e);
                let backoff = Duration::from_millis(10 * u64::from(attempt + 1));
                thread::sleep(backoff);
            }
            Err(e) => {
                return Err(format!(
                    "open named pipe `{pipe_path}` failed: {e}"
                ));
            }
        }
    }
    Err(format!(
        "named pipe `{pipe_path}` not available after 10 retries (~550ms): {}",
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "no underlying error captured".into())
    ))
}

#[cfg(not(any(unix, windows)))]
fn open_ipc_channel(_cli: &Cli) -> Result<Box<dyn ReadWrite>, String> {
    Err("unsupported platform: no IPC backend".into())
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

// ─── frame I/O ─────────────────────────────────────────────────────

#[derive(Debug)]
struct Request {
    mangled: String,
    args: Vec<u8>,
}

fn send_handshake<W: Write>(w: &mut W, schema_hash: &str) -> Result<(), String> {
    if schema_hash.len() as u32 != SCHEMA_HASH_LEN {
        return Err(format!(
            "internal: schema_hash length {} != {SCHEMA_HASH_LEN}",
            schema_hash.len()
        ));
    }
    let mut buf = Vec::with_capacity(4 * 3 + SCHEMA_HASH_LEN as usize);
    buf.extend_from_slice(&FRAME_MAGIC.to_le_bytes());
    buf.extend_from_slice(&SCHEMA_HASH_LEN.to_le_bytes());
    buf.extend_from_slice(&ABI_VERSION.to_le_bytes());
    buf.extend_from_slice(schema_hash.as_bytes());
    w.write_all(&buf).map_err(|e| format!("handshake send: {e}"))?;
    w.flush().map_err(|e| format!("handshake flush: {e}"))?;
    Ok(())
}

/// Returns `Ok(None)` on EOF or magic=0 (graceful shutdown).
fn read_request_frame<R: Read>(r: &mut R) -> Result<Option<Request>, String> {
    let mut header = [0u8; 12];
    match r.read_exact(&mut header) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(format!("request header read: {e}")),
    }
    let magic = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let mangled_len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let args_len = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;

    if magic == 0 {
        return Ok(None); // graceful shutdown signal
    }
    if magic != FRAME_MAGIC {
        return Err(format!(
            "bad request magic: got 0x{magic:08x}, want 0x{FRAME_MAGIC:08x}"
        ));
    }
    const MAX_PAYLOAD: usize = 256 * 1024 * 1024;
    if mangled_len > MAX_PAYLOAD || args_len > MAX_PAYLOAD {
        return Err(format!(
            "frame too large: mangled={mangled_len}B args={args_len}B"
        ));
    }

    let mut mangled_buf = vec![0u8; mangled_len];
    r.read_exact(&mut mangled_buf)
        .map_err(|e| format!("mangled name read: {e}"))?;
    let mangled = String::from_utf8(mangled_buf)
        .map_err(|e| format!("mangled name not UTF-8: {e}"))?;

    let mut args = vec![0u8; args_len];
    r.read_exact(&mut args)
        .map_err(|e| format!("args read: {e}"))?;

    Ok(Some(Request { mangled, args }))
}

fn write_response_frame<W: Write>(
    w: &mut W,
    status: i32,
    response: &[u8],
    detail: &str,
) -> Result<(), String> {
    let ret_len = response.len() as u32;
    let detail_len = detail.len() as u32;
    let mut buf = Vec::with_capacity(4 + 4 + 4 + 4 + response.len() + detail.len());
    buf.extend_from_slice(&FRAME_MAGIC.to_le_bytes());
    buf.extend_from_slice(&status.to_le_bytes());
    buf.extend_from_slice(&ret_len.to_le_bytes());
    buf.extend_from_slice(&detail_len.to_le_bytes());
    buf.extend_from_slice(response);
    buf.extend_from_slice(detail.as_bytes());
    w.write_all(&buf).map_err(|e| format!("response send: {e}"))?;
    w.flush().map_err(|e| format!("response flush: {e}"))?;
    Ok(())
}

// ─── request dispatch ──────────────────────────────────────────────

fn handle_request(
    lib: &libloading::Library,
    cache: &mut HashMap<String, WrapperFn>,
    ret_buf: &mut Vec<u8>,
    req: &Request,
) -> (i32, Vec<u8>, String) {
    // Resolve symbol (cached).
    let wrapper = match cache.get(&req.mangled).copied() {
        Some(f) => f,
        None => match resolve_wrapper(lib, &req.mangled) {
            Ok(f) => {
                cache.insert(req.mangled.clone(), f);
                f
            }
            Err(detail) => return (LEO4_ERR_RUST_DLSYM_FAILED, Vec::new(), detail),
        },
    };

    // Call wrapper with the shared response buffer; grow on too-small.
    invoke_wrapper(wrapper, &req.args, ret_buf)
}

fn resolve_wrapper(
    lib: &libloading::Library,
    mangled: &str,
) -> Result<WrapperFn, String> {
    let mut symname = mangled.as_bytes().to_owned();
    symname.push(0); // NUL terminator for libloading
    // SAFETY: the loaded cdylib emitted this symbol via the
    // `#[leo4::export]` proc-macro (Phase 9-1); its signature is
    // the canonical-ABI WrapperFn.
    let raw: libloading::Symbol<'_, WrapperFn> = unsafe {
        lib.get(&symname)
            .map_err(|e| format!("dlsym `{mangled}`: {e}"))?
    };
    // SAFETY: copying the function pointer out of the libloading
    // Symbol wrapper. The pointer remains valid for the cdylib's
    // lifetime.
    Ok(unsafe { *raw.into_raw() })
}

fn invoke_wrapper(
    wrapper: WrapperFn,
    args: &[u8],
    ret_buf: &mut Vec<u8>,
) -> (i32, Vec<u8>, String) {
    // Wrap the wrapper call in `catch_unwind`. The wrapper itself
    // already catches panics inside the user fn (Phase 9-1) and
    // returns LEO4_ERR_RUST_PANIC, so this outer guard only
    // matters for panics from the wrapper plumbing itself
    // (e.g. accidental allocator-OOM unwind). On panic we report
    // LEO4_ERR_RUST_PANIC and exit the worker; the dispatcher
    // sees EOF on the IPC channel and respawns.
    let mut ret_len: usize = 0;
    let call_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        loop {
            let args_ptr = args.as_ptr();
            let args_len = args.len();
            let ret_ptr = ret_buf.as_mut_ptr();
            let ret_cap = ret_buf.len();
            ret_len = 0;
            // SAFETY: wrapper signature is the canonical-ABI shape.
            let rc = unsafe {
                wrapper(args_ptr, args_len, ret_ptr, ret_cap, &raw mut ret_len)
            };
            if rc == LEO4_ERR_BUFFER_TOO_SMALL {
                // Wrapper wrote the required size into ret_len.
                if ret_len <= ret_cap {
                    // Bogus retry signal; treat as decode error.
                    return (LEO4_ERR_DECODE, 0usize, String::from(
                        "wrapper returned BUFFER_TOO_SMALL but ret_len <= ret_cap",
                    ));
                }
                ret_buf.resize(ret_len, 0);
                continue;
            }
            return (rc, ret_len, String::new());
        }
    }));

    if let Ok((rc, used, detail)) = call_result {
        let body = if rc == LEO4_OK {
            ret_buf[..used].to_vec()
        } else {
            Vec::new()
        };
        (rc, body, detail)
    } else {
        let _ = std::io::stderr().flush();
        // The wrapper's own catch_unwind should have caught
        // any user-fn panic. If we are here, the panic
        // originated outside the wrapper plumbing. Try to
        // emit a final response with the panic code and
        // then abort so the dispatcher respawns.
        // The dispatcher reads EOF after the message;
        // since the call frame won't actually be sent (we
        // can't safely return to the request loop), we
        // abort here.
        process::abort();
    }
}

// ─── unit tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_base32_matches_known_input() {
        // Sanity: same algorithm as `leo4-rust-emit`. Spot-check
        // a fixed string lands on a 13-char base32lc digest.
        let h = fnv1a64_base32lc("package leo4-sample; interface Sample {  }");
        assert_eq!(h.len(), 13);
        assert!(h.chars().all(|c| BASE32_LC.contains(&(c as u8))));
    }

    #[test]
    fn surface_form_table() {
        assert_eq!(surface_form("u64"), "u64");
        assert_eq!(surface_form("str"), "string");
        assert_eq!(surface_form("b"), "bool");
        assert_eq!(surface_form("L_u32_l"), "list<u32>");
        assert_eq!(surface_form("O_str_o"), "option<string>");
        assert_eq!(surface_form("S_Sample_Point_s"), "Sample.Point");
        // Unknown mangles fall through verbatim.
        assert_eq!(surface_form("???"), "???");
    }

    #[test]
    fn handshake_frame_round_trips_through_pipe() {
        // Use an in-memory `Cursor<Vec<u8>>` to verify the
        // wire format produced by send_handshake.
        let mut buf: Vec<u8> = Vec::new();
        send_handshake(&mut buf, "abcdefghijklm").unwrap();
        assert_eq!(buf.len(), 12 + 13);
        let magic = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let hlen = u32::from_le_bytes(buf[4..8].try_into().unwrap());
        let abi = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        assert_eq!(magic, FRAME_MAGIC);
        assert_eq!(hlen, SCHEMA_HASH_LEN);
        assert_eq!(abi, ABI_VERSION);
        assert_eq!(&buf[12..25], b"abcdefghijklm");
    }

    #[test]
    fn read_request_frame_handles_magic_zero_shutdown() {
        let mut buf: Vec<u8> = Vec::new();
        // magic=0, mangled_len=0, args_len=0 → graceful shutdown.
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        let mut c = std::io::Cursor::new(buf);
        let req = read_request_frame(&mut c).unwrap();
        assert!(req.is_none());
    }

    #[test]
    fn read_request_frame_round_trips_a_call() {
        let mut buf: Vec<u8> = Vec::new();
        let mangled = "leo4_rust__add__u64_u64";
        let args = b"\x01\x02\x03\x04\x05\x06\x07\x08";
        buf.extend_from_slice(&FRAME_MAGIC.to_le_bytes());
        buf.extend_from_slice(&(mangled.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(args.len() as u32).to_le_bytes());
        buf.extend_from_slice(mangled.as_bytes());
        buf.extend_from_slice(args);
        let mut c = std::io::Cursor::new(buf);
        let req = read_request_frame(&mut c).unwrap().expect("frame");
        assert_eq!(req.mangled, mangled);
        assert_eq!(req.args, args.as_slice());
    }

    #[test]
    fn write_response_frame_emits_expected_layout() {
        let mut buf: Vec<u8> = Vec::new();
        write_response_frame(&mut buf, 0, b"\xff\xee", "").unwrap();
        let magic = u32::from_le_bytes(buf[0..4].try_into().unwrap());
        let status = i32::from_le_bytes(buf[4..8].try_into().unwrap());
        let ret_len = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let det_len = u32::from_le_bytes(buf[12..16].try_into().unwrap());
        assert_eq!(magic, FRAME_MAGIC);
        assert_eq!(status, 0);
        assert_eq!(ret_len, 2);
        assert_eq!(det_len, 0);
        assert_eq!(&buf[16..18], &[0xff, 0xee]);
    }
}

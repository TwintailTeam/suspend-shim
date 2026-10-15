#![windows_subsystem = "windows"]

use std::ffi::{OsStr, OsString, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

const LAUNCHER_LANG: &str = "en-us";
const LAUNCHER_SUB_CHANNEL: &str = "6";
const TRACKING_CODE: &str = "";
const CHANNEL_ARGS: &str = "";
const INFINITE: u32 = 0xFFFF_FFFF;
const CREATE_SUSPENDED: u32 = 0x4;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x8_0000;
const PROC_THREAD_ATTRIBUTE_PARENT_PROCESS: usize = 0x2_0000;
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
const JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK: u32 = 0x1000;

type Handle = *mut c_void;

#[repr(C)]
struct StartupInfoW { cb: u32, reserved: *mut u16, desktop: *mut u16, title: *mut u16, pos_size_fill_flags: [u32; 8], show_window: u16, reserved2_len: u16, reserved2: *mut u8, std_handles: [Handle; 3] }

#[repr(C)]
struct StartupInfoExW { startup_info: StartupInfoW, attr_list: *mut c_void }

#[repr(C)]
struct ProcessInformation { process: Handle, thread: Handle, process_id: u32, thread_id: u32 }

#[repr(C)]
struct JobBasicLimit { per_process_time: i64, per_job_time: i64, limit_flags: u32, min_ws: usize, max_ws: usize, active_process_limit: u32, affinity: usize, priority_class: u32, scheduling_class: u32 }

#[repr(C)]
struct IoCounters { read_op: u64, write_op: u64, other_op: u64, read_tx: u64, write_tx: u64, other_tx: u64 }

#[repr(C)]
struct JobExtendedLimit { basic: JobBasicLimit, io: IoCounters, process_mem: usize, job_mem: usize, peak_process_mem: usize, peak_job_mem: usize }

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: *const c_void, ta: *const c_void, inherit: i32, flags: u32, env: *const c_void, dir: *const u16, si: *const StartupInfoW, pi: *mut ProcessInformation) -> i32;
    fn CreateJobObjectW(sa: *const c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(job: Handle, class: i32, info: *const c_void, len: u32) -> i32;
    fn InitializeProcThreadAttributeList(list: *mut c_void, count: u32, flags: u32, size: *mut usize) -> i32;
    fn UpdateProcThreadAttribute(list: *mut c_void, flags: u32, attr: usize, value: *const c_void, size: usize, prev: *mut c_void, ret: *mut usize) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn WaitForSingleObject(handle: Handle, ms: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
    fn TerminateProcess(process: Handle, code: u32) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetLastError() -> u32;
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> *const c_void;
    fn GetProcessHeap() -> Handle;
    fn HeapFree(heap: Handle, flags: u32, mem: *mut c_void) -> i32;
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

fn unix_to_dos(path: &OsStr) -> Option<PathBuf> {
    let kernel32 = unsafe { GetModuleHandleW(wide(OsStr::new("kernel32")).as_ptr()) };
    let proc = unsafe { GetProcAddress(kernel32, c"wine_get_dos_file_name".as_ptr().cast()) };
    if kernel32.is_null() || proc.is_null() { return None; }
    let convert: unsafe extern "system" fn(*const u8) -> *mut u16 = unsafe { std::mem::transmute(proc) };

    let mut unix = path.to_string_lossy().into_owned().into_bytes();
    unix.push(0);
    let dos = unsafe { convert(unix.as_ptr()) };
    if dos.is_null() { return None; }
    let len = (0..).take_while(|&i| unsafe { *dos.add(i) } != 0).count();
    let result = OsString::from_wide(unsafe { std::slice::from_raw_parts(dos, len) });
    unsafe { HeapFree(GetProcessHeap(), 0, dos.cast()) };
    Some(PathBuf::from(result))
}

fn resolve_path(arg: &OsStr, must_be_file: bool) -> Result<PathBuf, String> {
    let path = if arg.to_string_lossy().starts_with('/') {
        unix_to_dos(arg).ok_or_else(|| format!("cannot convert unix path {}", arg.display()))?
    } else {
        PathBuf::from(arg)
    };
    let path = std::path::absolute(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let path = PathBuf::from(path.to_string_lossy().replace('/', "\\"));
    if must_be_file && !path.is_file() { return Err(format!("not found: {}", path.display())); }
    Ok(path)
}

fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) { return arg.to_owned(); }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => { out.extend(std::iter::repeat_n('\\', backslashes + 1)); backslashes = 0; }
            _ => backslashes = 0,
        }
        out.push(c);
    }
    out.extend(std::iter::repeat_n('\\', backslashes));
    out.push('"');
    out
}

fn build_command_line(game: &Path, extra: &[OsString]) -> String {
    let extra_args = extra.iter().map(|a| quote_arg(&a.to_string_lossy())).collect::<Vec<_>>().join(" ");

    let mut optional = String::new();
    if !TRACKING_CODE.is_empty() { optional += &format!(" -tracking_code=\"{TRACKING_CODE}\""); }
    if !LAUNCHER_SUB_CHANNEL.is_empty() { optional += &format!(" -launcher_sub_channel={LAUNCHER_SUB_CHANNEL}"); }

    format!("\"{}\"  -launcher_lang={LAUNCHER_LANG} {extra_args} {optional} {CHANNEL_ARGS}", game.display())
}

// Start the genuine signed launcher suspended; it is never resumed, so its DLL/Qt tree is never
// loaded. It exists only as the game's parent so OneChannelLC's signature/fingerprint check sees
// a real Gryph Frontier-signed parent image.
fn spawn_suspended_launcher(launcher: &Path) -> Result<ProcessInformation, String> {
    let app_w = wide(launcher.as_os_str());
    let dir_w = launcher.parent().map(|p| wide(p.as_os_str()));
    let mut si: StartupInfoW = unsafe { std::mem::zeroed() };
    si.cb = size_of::<StartupInfoW>() as u32;
    let mut pi = ProcessInformation { process: null_mut(), thread: null_mut(), process_id: 0, thread_id: 0 };
    let ok = unsafe { CreateProcessW(app_w.as_ptr(), null_mut(), null(), null(), 0, CREATE_SUSPENDED, null(), dir_w.as_ref().map_or(null(), |d| d.as_ptr()), &si, &mut pi) };
    if ok == 0 { return Err(format!("launcher CreateProcessW failed, error {} ({})", unsafe { GetLastError() }, launcher.display())); }
    Ok(pi)
}

// Kill-on-close job so the suspended launcher dies with us even on a hard exit.
fn make_cleanup_job(child: Handle) -> Handle {
    let job = unsafe { CreateJobObjectW(null(), null()) };
    if job.is_null() { return null_mut(); }
    let mut info: JobExtendedLimit = unsafe { std::mem::zeroed() };
    info.basic.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK;
    unsafe {
        SetInformationJobObject(job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION, (&info as *const JobExtendedLimit).cast(), size_of::<JobExtendedLimit>() as u32);
        let assign: unsafe extern "system" fn(Handle, Handle) -> i32 = std::mem::transmute(GetProcAddress(GetModuleHandleW(wide(OsStr::new("kernel32")).as_ptr()), c"AssignProcessToJobObject".as_ptr().cast()));
        assign(job, child);
    }
    job
}

fn spawn_game_reparented(game: &Path, work_dir: &Path, command_line: &str, parent: Handle) -> Result<ProcessInformation, String> {
    let app_w = wide(game.as_os_str());
    let mut cmd_w = wide(OsStr::new(command_line));
    let dir_w = wide(work_dir.as_os_str());

    // Size, allocate, and fill a proc-thread attribute list carrying the parent-process override.
    let mut size: usize = 0;
    unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size) };
    let mut buf = vec![0u8; size];
    let list = buf.as_mut_ptr().cast::<c_void>();
    if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 {
        return Err(format!("InitializeProcThreadAttributeList failed, error {}", unsafe { GetLastError() }));
    }
    let parent_box = parent;
    if unsafe { UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_PARENT_PROCESS, (&parent_box as *const Handle).cast(), size_of::<Handle>(), null_mut(), null_mut()) } == 0 {
        unsafe { DeleteProcThreadAttributeList(list) };
        return Err(format!("UpdateProcThreadAttribute failed, error {}", unsafe { GetLastError() }));
    }

    let mut siex: StartupInfoExW = unsafe { std::mem::zeroed() };
    siex.startup_info.cb = size_of::<StartupInfoExW>() as u32;
    siex.attr_list = list;
    let mut pi = ProcessInformation { process: null_mut(), thread: null_mut(), process_id: 0, thread_id: 0 };
    let ok = unsafe { CreateProcessW(app_w.as_ptr(), cmd_w.as_mut_ptr(), null(), null(), 0, EXTENDED_STARTUPINFO_PRESENT, null(), dir_w.as_ptr(), (&siex as *const StartupInfoExW).cast(), &mut pi) };
    let err = unsafe { GetLastError() };
    unsafe { DeleteProcThreadAttributeList(list) };
    if ok == 0 { return Err(format!("game CreateProcessW failed, error {err}")); }
    Ok(pi)
}

struct Opts { launcher: PathBuf, game: PathBuf, extra: Vec<OsString> }

fn parse_args() -> Result<Opts, String> {
    let mut launcher: Option<OsString> = None;
    let mut game: Option<OsString> = None;
    let mut extra: Vec<OsString> = Vec::new();
    let mut args = std::env::args_os().skip(1);
    let mut rest = false;
    while let Some(a) = args.next() {
        if rest { extra.push(a); continue; }
        match a.to_string_lossy().as_ref() {
            "--launcher" => launcher = Some(args.next().ok_or("--launcher needs a path")?),
            "--" => rest = true,
            _ if game.is_none() => game = Some(a),
            _ => extra.push(a),
        }
    }
    let launcher = launcher.ok_or("usage: suspend_start.exe --launcher <real Games.exe> <game exe> [args...]")?;
    let game = game.ok_or("usage: suspend_start.exe --launcher <real Games.exe> <game exe> [args...]")?;
    Ok(Opts { launcher: resolve_path(&launcher, true)?, game: resolve_path(&game, true)?, extra })
}

fn run() -> Result<u32, String> {
    let opts = parse_args()?;
    let work_dir = opts.game.parent().ok_or("game path has no parent directory")?.to_path_buf();
    let command_line = build_command_line(&opts.game, &opts.extra);
    eprintln!("hg_helper: parent={} cmd={command_line}", opts.launcher.display());

    let launcher = spawn_suspended_launcher(&opts.launcher)?;
    let job = make_cleanup_job(launcher.process);

    let game = match spawn_game_reparented(&opts.game, &work_dir, &command_line, launcher.process) {
        Ok(pi) => pi,
        Err(e) => {
            unsafe { TerminateProcess(launcher.process, 1); CloseHandle(launcher.thread); CloseHandle(launcher.process); if !job.is_null() { CloseHandle(job); } }
            return Err(e);
        }
    };

    let mut exit_code = 0;
    unsafe {
        WaitForSingleObject(game.process, INFINITE);
        GetExitCodeProcess(game.process, &mut exit_code);
        CloseHandle(game.thread);
        CloseHandle(game.process);
        TerminateProcess(launcher.process, 0);
        CloseHandle(launcher.thread);
        CloseHandle(launcher.process);
        if !job.is_null() { CloseHandle(job); }
    }
    Ok(exit_code)
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code as i32),
        Err(e) => { eprintln!("hg_helper: {e}"); std::process::exit(1); }
    }
}

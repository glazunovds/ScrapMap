//! Development-only hardware watchpoint probe. The target is never modified.
//! Usage: native_watchpoint <pid> <address> [max-hits]

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
fn main() {
    eprintln!("Windows x64 only");
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn main() -> Result<(), String> {
    use std::{
        env,
        mem::zeroed,
        time::{Duration, Instant},
    };
    use windows::Win32::{
        Foundation::{CloseHandle, DBG_CONTINUE, DBG_EXCEPTION_NOT_HANDLED, EXCEPTION_SINGLE_STEP},
        System::{
            Diagnostics::Debug::{
                ContinueDebugEvent, DebugActiveProcess, DebugActiveProcessStop,
                DebugSetProcessKillOnExit, GetThreadContext, SetThreadContext, WaitForDebugEvent,
                CONTEXT, CONTEXT_ALL_AMD64, DEBUG_EVENT, EXCEPTION_DEBUG_EVENT,
            },
            Threading::{
                OpenThread, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION, THREAD_SET_CONTEXT,
                THREAD_SUSPEND_RESUME,
            },
        },
    };

    let mut args = env::args().skip(1);
    let pid = args
        .next()
        .ok_or("missing pid")?
        .parse::<u32>()
        .map_err(|_| "invalid pid")?;
    let address_text = args.next().ok_or("missing address")?;
    let address = usize::from_str_radix(address_text.trim_start_matches("0x"), 16)
        .map_err(|_| "invalid address")?;
    let max_hits = args
        .next()
        .map(|v| v.parse::<usize>())
        .transpose()
        .map_err(|_| "invalid max-hits")?
        .unwrap_or(32);

    unsafe { DebugActiveProcess(pid).map_err(|e| format!("DebugActiveProcess: {e}"))? };
    let _guard = DebugDetach(pid);
    unsafe { DebugSetProcessKillOnExit(false) }
        .map_err(|e| format!("DebugSetProcessKillOnExit: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut hits = 0usize;
    let mut armed_existing_threads = false;

    while Instant::now() < deadline && hits < max_hits {
        let mut event: DEBUG_EVENT = unsafe { zeroed() };
        if unsafe { WaitForDebugEvent(&mut event, 1000) }.is_err() {
            continue;
        }

        if !armed_existing_threads {
            let armed = arm_all_threads(pid, address);
            eprintln!("armed {armed} existing threads");
            armed_existing_threads = true;
        }

        let mut status = DBG_CONTINUE;
        if event.dwDebugEventCode == EXCEPTION_DEBUG_EVENT {
            let exception = unsafe { event.u.Exception };
            let code = exception.ExceptionRecord.ExceptionCode;
            if code == EXCEPTION_SINGLE_STEP {
                if let Ok(mut context) = thread_context(event.dwThreadId) {
                    hits += 1;
                    println!(
                        "hit={hits} tid={} rip={:016X} rax={:016X} rbx={:016X} rcx={:016X} rdx={:016X} rsi={:016X} rdi={:016X} rbp={:016X} rsp={:016X} r8={:016X} r9={:016X} r10={:016X} r11={:016X} r12={:016X} r13={:016X} r14={:016X} r15={:016X}",
                        event.dwThreadId, context.Rip, context.Rax, context.Rbx, context.Rcx,
                        context.Rdx, context.Rsi, context.Rdi, context.Rbp, context.Rsp,
                        context.R8, context.R9, context.R10, context.R11, context.R12,
                        context.R13, context.R14, context.R15
                    );
                    context.Dr6 = 0;
                    let _ = set_thread_context(event.dwThreadId, &context);
                }
            } else {
                status = DBG_EXCEPTION_NOT_HANDLED;
            }
        }

        let _ = arm_thread(event.dwThreadId, address);
        unsafe { ContinueDebugEvent(event.dwProcessId, event.dwThreadId, status) }
            .map_err(|e| format!("ContinueDebugEvent: {e}"))?;
    }
    eprintln!("captured {hits} hits");
    Ok(())
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn open_thread(tid: u32) -> windows::core::Result<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::System::Threading::{
        OpenThread, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION, THREAD_SET_CONTEXT,
        THREAD_SUSPEND_RESUME,
    };
    unsafe {
        OpenThread(
            THREAD_GET_CONTEXT
                | THREAD_SET_CONTEXT
                | THREAD_QUERY_INFORMATION
                | THREAD_SUSPEND_RESUME,
            false,
            tid,
        )
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn thread_context(tid: u32) -> Result<windows::Win32::System::Diagnostics::Debug::CONTEXT, String> {
    use std::mem::zeroed;
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Diagnostics::Debug::{GetThreadContext, CONTEXT, CONTEXT_ALL_AMD64},
    };
    let thread = open_thread(tid).map_err(|e| e.to_string())?;
    #[repr(C, align(16))]
    struct AlignedContext(CONTEXT);
    let mut context = AlignedContext(unsafe { zeroed() });
    context.0.ContextFlags = CONTEXT_ALL_AMD64;
    let result = unsafe { GetThreadContext(thread, &mut context.0) }.map_err(|e| e.to_string());
    unsafe {
        let _ = CloseHandle(thread);
    }
    result.map(|_| context.0)
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn set_thread_context(
    tid: u32,
    context: &windows::Win32::System::Diagnostics::Debug::CONTEXT,
) -> Result<(), String> {
    use windows::Win32::{Foundation::CloseHandle, System::Diagnostics::Debug::SetThreadContext};
    let thread = open_thread(tid).map_err(|e| e.to_string())?;
    #[repr(C, align(16))]
    struct AlignedContext(windows::Win32::System::Diagnostics::Debug::CONTEXT);
    let aligned = AlignedContext(*context);
    let result = unsafe { SetThreadContext(thread, &aligned.0) }.map_err(|e| e.to_string());
    unsafe {
        let _ = CloseHandle(thread);
    }
    result
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn arm_thread(tid: u32, address: usize) -> Result<(), String> {
    let mut context = thread_context(tid)?;
    context.Dr0 = address as u64;
    context.Dr6 = 0;
    context.Dr7 = (context.Dr7 & !0x000F_0003) | 0x000F_0001;
    set_thread_context(tid, &context)
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn arm_all_threads(pid: u32, address: usize) -> usize {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
        },
    };

    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) } {
        Ok(snapshot) => snapshot,
        Err(_) => return 0,
    };
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut armed = 0;
    let mut has_entry = unsafe { Thread32First(snapshot, &mut entry) }.is_ok();
    while has_entry {
        if entry.th32OwnerProcessID == pid {
            match arm_thread(entry.th32ThreadID, address) {
                Ok(()) => armed += 1,
                Err(error) => eprintln!("thread {}: {error}", entry.th32ThreadID),
            }
        }
        has_entry = unsafe { Thread32Next(snapshot, &mut entry) }.is_ok();
    }
    unsafe {
        let _ = CloseHandle(snapshot);
    }
    armed
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
struct DebugDetach(u32);

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
impl Drop for DebugDetach {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Diagnostics::Debug::DebugActiveProcessStop(self.0);
        }
    }
}

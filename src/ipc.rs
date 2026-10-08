//! Named kernel objects shared between `start`, `stop` and the daemon.
//!
//! * `Alive` mutex: owned by the running daemon; waiting on it returns once the daemon exits.
//! * `Stop` event: signalled by `stop` to ask the daemon to tear down.
//!
//! Objects get a permissive DACL with a low integrity label, so elevated and
//! non-elevated invocations (Sunshine's per-command "elevated" flag) can talk to each other.

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HLOCAL, LocalFree, WAIT_ABANDONED,
    WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, OpenEventW, OpenMutexW, ReleaseMutex,
    ResetEvent, SYNCHRONIZATION_SYNCHRONIZE, SetEvent, WaitForSingleObject,
};
use windows::core::{PCWSTR, w};

const ALIVE_NAME: PCWSTR = w!("Local\\SunshineVDisplay.Alive");
const STOP_NAME: PCWSTR = w!("Local\\SunshineVDisplay.Stop");

pub struct Handle(pub HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct Sa {
    sd: PSECURITY_DESCRIPTOR,
    attrs: SECURITY_ATTRIBUTES,
}

impl Sa {
    fn everyone() -> Option<Self> {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                w!("D:(A;;GA;;;WD)(A;;GA;;;SY)S:(ML;;NW;;;LW)"),
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .ok()?;
        }
        Some(Sa {
            sd,
            attrs: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd.0,
                bInheritHandle: false.into(),
            },
        })
    }
}

impl Drop for Sa {
    fn drop(&mut self) {
        unsafe {
            LocalFree(Some(HLOCAL(self.sd.0)));
        }
    }
}

/// Daemon side: claims the alive mutex and creates a fresh stop event.
pub fn daemon_objects() -> Result<(Handle, Handle), String> {
    let sa = Sa::everyone();
    let attrs = sa.as_ref().map(|s| &s.attrs as *const SECURITY_ATTRIBUTES);
    unsafe {
        let alive =
            CreateMutexW(attrs, true, ALIVE_NAME).map_err(|e| format!("CreateMutex: {e}"))?;
        let alive = Handle(alive);
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Another daemon still holds it: wait for it to release (it may be tearing down).
            let r = WaitForSingleObject(alive.0, 10_000);
            if r != WAIT_OBJECT_0 && r != WAIT_ABANDONED {
                return Err("another vdisplay daemon is already running".into());
            }
        }
        let stop = Handle(
            CreateEventW(attrs, true, false, STOP_NAME).map_err(|e| format!("CreateEvent: {e}"))?,
        );
        let _ = ResetEvent(stop.0);
        Ok((alive, stop))
    }
}

/// Returns true if a daemon is currently running.
pub fn daemon_running() -> bool {
    unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, ALIVE_NAME) }
        .map(Handle)
        .is_ok()
}

/// Signals the daemon to stop and waits up to `timeout_ms` for it to exit.
/// Returns Ok(false) if no daemon was running.
pub fn stop_daemon(timeout_ms: u32) -> Result<bool, String> {
    unsafe {
        let Ok(alive) = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, ALIVE_NAME) else {
            return Ok(false);
        };
        let alive = Handle(alive);
        let stop = OpenEventW(EVENT_MODIFY_STATE, false, STOP_NAME)
            .map_err(|e| format!("OpenEvent: {e}"))?;
        let stop = Handle(stop);
        SetEvent(stop.0).map_err(|e| format!("SetEvent: {e}"))?;
        match WaitForSingleObject(alive.0, timeout_ms) {
            WAIT_OBJECT_0 | WAIT_ABANDONED => {
                let _ = ReleaseMutex(alive.0);
                Ok(true)
            }
            _ => Err("timed out waiting for the vdisplay daemon to exit".into()),
        }
    }
}

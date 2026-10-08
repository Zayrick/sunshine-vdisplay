//! Minimal user-mode client for the SudoVDA virtual display driver.
//! Mirrors `third-party/sudovda/sudovda.h` used by Apollo.

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, LUID};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::{GUID, PCWSTR};

// {e5bcc234-1e0c-418a-a0d4-ef8b7501414d}
const SUVDA_INTERFACE_GUID: GUID = GUID::from_u128(0xe5bcc234_1e0c_418a_a0d4_ef8b7501414d);

/// Protocol version this client was written against (see sudovda-ioctl.h).
const PROTOCOL_MAJOR: u8 = 0;
const PROTOCOL_MINOR: u8 = 2;

const fn ctl_code(function: u32) -> u32 {
    // CTL_CODE(FILE_DEVICE_UNKNOWN, function, METHOD_BUFFERED, FILE_ANY_ACCESS)
    (0x22 << 16) | (function << 2)
}

const IOCTL_ADD_VIRTUAL_DISPLAY: u32 = ctl_code(0x800);
const IOCTL_REMOVE_VIRTUAL_DISPLAY: u32 = ctl_code(0x801);
const IOCTL_SET_RENDER_ADAPTER: u32 = ctl_code(0x802);
const IOCTL_GET_WATCHDOG: u32 = ctl_code(0x803);
const IOCTL_DRIVER_PING: u32 = ctl_code(0x888);
const IOCTL_GET_PROTOCOL_VERSION: u32 = ctl_code(0x8FF);

#[repr(C)]
struct AddParams {
    width: u32,
    height: u32,
    refresh_rate: u32,
    monitor_guid: GUID,
    device_name: [u8; 14],
    serial_number: [u8; 14],
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct AddOut {
    pub adapter_luid: LUID,
    pub target_id: u32,
}

#[repr(C)]
#[derive(Default)]
pub struct Watchdog {
    pub timeout: u32,
    pub countdown: u32,
}

#[repr(C)]
#[derive(Default)]
struct ProtocolVersion {
    major: u8,
    minor: u8,
    incremental: u8,
    test_build: u8,
}

pub struct Driver {
    handle: HANDLE,
}

// The handle is only used for synchronous DeviceIoControl calls, which the
// I/O manager serializes for us.
unsafe impl Send for Driver {}
unsafe impl Sync for Driver {}

impl Driver {
    pub fn open() -> Result<Self, String> {
        let path = find_device_path()?;
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map_err(|e| format!("cannot open SudoVDA device: {e}"))?;
        let driver = Driver { handle };

        let mut version = ProtocolVersion::default();
        driver
            .ioctl(IOCTL_GET_PROTOCOL_VERSION, None::<&()>, Some(&mut version))
            .map_err(|e| format!("cannot query driver protocol version: {e}"))?;
        // Same rule as SUDOVDA::isProtocolCompatible
        if version.major != PROTOCOL_MAJOR || PROTOCOL_MINOR > version.minor {
            return Err(format!(
                "incompatible SudoVDA driver protocol {}.{}.{} (expected {PROTOCOL_MAJOR}.{PROTOCOL_MINOR}.x)",
                version.major, version.minor, version.incremental
            ));
        }
        Ok(driver)
    }

    fn ioctl<I, O>(
        &self,
        code: u32,
        input: Option<&I>,
        output: Option<&mut O>,
    ) -> windows::core::Result<()> {
        let mut returned = 0u32;
        unsafe {
            DeviceIoControl(
                self.handle,
                code,
                input.map(|i| i as *const I as *const c_void),
                if input.is_some() {
                    size_of::<I>() as u32
                } else {
                    0
                },
                output.map(|o| o as *mut O as *mut c_void),
                if size_of::<O>() > 0 {
                    size_of::<O>() as u32
                } else {
                    0
                },
                Some(&mut returned),
                None,
            )
        }
    }

    pub fn add_display(
        &self,
        width: u32,
        height: u32,
        refresh_mhz: u32,
        guid: GUID,
        device_name: &str,
        serial: &str,
    ) -> windows::core::Result<AddOut> {
        let mut params = AddParams {
            width,
            height,
            refresh_rate: refresh_mhz,
            monitor_guid: guid,
            device_name: [0; 14],
            serial_number: [0; 14],
        };
        copy_cstr(&mut params.device_name, device_name);
        copy_cstr(&mut params.serial_number, serial);
        let mut out = AddOut::default();
        self.ioctl(IOCTL_ADD_VIRTUAL_DISPLAY, Some(&params), Some(&mut out))?;
        Ok(out)
    }

    pub fn remove_display(&self, guid: GUID) -> windows::core::Result<()> {
        self.ioctl(IOCTL_REMOVE_VIRTUAL_DISPLAY, Some(&guid), None::<&mut ()>)
    }

    pub fn set_render_adapter(&self, luid: LUID) -> windows::core::Result<()> {
        self.ioctl(IOCTL_SET_RENDER_ADAPTER, Some(&luid), None::<&mut ()>)
    }

    pub fn watchdog(&self) -> windows::core::Result<Watchdog> {
        let mut out = Watchdog::default();
        self.ioctl(IOCTL_GET_WATCHDOG, None::<&()>, Some(&mut out))?;
        Ok(out)
    }

    pub fn ping(&self) -> windows::core::Result<()> {
        self.ioctl(IOCTL_DRIVER_PING, None::<&()>, None::<&mut ()>)
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Copies an ASCII string into a fixed, NUL-terminated buffer (max 13 chars),
/// like the `strncpy(.., 13)` in sudovda.h.
fn copy_cstr(dst: &mut [u8; 14], src: &str) {
    for (d, s) in dst.iter_mut().zip(
        src.bytes()
            .filter(|b| b.is_ascii_graphic() || *b == b' ')
            .take(13),
    ) {
        *d = s;
    }
}

fn find_device_path() -> Result<Vec<u16>, String> {
    unsafe {
        let set = SetupDiGetClassDevsW(
            Some(&SUVDA_INTERFACE_GUID),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
        .map_err(|e| format!("SetupDiGetClassDevs failed: {e}"))?;

        let mut result = Err(
            "SudoVDA driver not found (is the SudoMaker Virtual Display Adapter installed?)"
                .to_string(),
        );
        let mut iface = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        if SetupDiEnumDeviceInterfaces(set, None, &SUVDA_INTERFACE_GUID, 0, &mut iface).is_ok() {
            let mut required = 0u32;
            let _ =
                SetupDiGetDeviceInterfaceDetailW(set, &iface, None, 0, Some(&mut required), None);
            // Allocate as u32 for alignment of the detail struct.
            let mut buf = vec![0u32; (required as usize).div_ceil(4).max(2)];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &iface, Some(detail), required, None, None)
                .is_ok()
            {
                let p = std::ptr::addr_of!((*detail).DevicePath) as *const u16;
                let len = (0..).take_while(|&i| *p.add(i) != 0).count();
                let mut path = std::slice::from_raw_parts(p, len).to_vec();
                path.push(0);
                result = Ok(path);
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
        result
    }
}

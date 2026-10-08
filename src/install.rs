//! Installs the bundled SudoVDA driver, mirroring Apollo's `drivers/sudovda/install.bat`:
//!
//! 1. trust `sudovda.cer` (LocalMachine `Root` + `TrustedPublisher`),
//! 2. create the root-enumerated device node `Root\SudoMaker\SudoVDA` (Display class),
//! 3. install `SudoVDA.inf` on it.
//!
//! Apollo shells out to `nefconc.exe` for 2 and 3; we call SetupAPI/newdev directly.
//! Uninstalling undoes all three: device node, driver store package and certificate.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DICD_GENERATE_ID, DIF_REGISTERDEVICE, DIF_REMOVE, DIGCF_PRESENT, GUID_DEVCLASS_DISPLAY,
    HDEVINFO, INSTALLFLAG_FORCE, SP_DEVINFO_DATA, SPDRP_HARDWAREID, SetupDiCallClassInstaller,
    SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW, SetupDiDestroyDeviceInfoList,
    SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW,
    SetupDiSetDeviceRegistryPropertyW, SetupUninstallOEMInfW, SUOI_FORCEDELETE,
    UpdateDriverForPlugAndPlayDevicesW,
};
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::Security::Cryptography::{
    CERT_FIND_EXISTING, CERT_OPEN_STORE_FLAGS, CERT_QUERY_ENCODING_TYPE,
    CERT_STORE_ADD_REPLACE_EXISTING, CERT_STORE_PROV_SYSTEM_W, CERT_SYSTEM_STORE_LOCAL_MACHINE,
    CertAddEncodedCertificateToStore, CertCloseStore, CertCreateCertificateContext,
    CertDeleteCertificateFromStore, CertFindCertificateInStore, CertFreeCertificateContext,
    CertOpenStore, HCERTSTORE, X509_ASN_ENCODING,
};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    IsUserAnAdmin, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::core::{BOOL, PCWSTR, w};

use crate::driver;
use crate::log;

const HARDWARE_ID: &str = r"Root\SudoMaker\SudoVDA";

const FILES: &[(&str, &[u8])] = &[
    ("SudoVDA.inf", include_bytes!("../drivers/sudovda/SudoVDA.inf")),
    ("SudoVDA.dll", include_bytes!("../drivers/sudovda/SudoVDA.dll")),
    ("sudovda.cat", include_bytes!("../drivers/sudovda/sudovda.cat")),
];
const CERT: &[u8] = include_bytes!("../drivers/sudovda/sudovda.cer");

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn driver_present() -> bool {
    driver::find_device_path().is_ok()
}

/// `vdisplay install-driver`: (re)installs the bundled driver, elevating if needed.
pub fn install() -> Result<(), String> {
    if is_elevated() {
        install_elevated()?;
    } else {
        run_elevated("install-driver")?;
    }
    // The device interface shows up asynchronously once the device has started.
    let deadline = Instant::now() + Duration::from_secs(15);
    while !driver_present() {
        if Instant::now() > deadline {
            return Err("SudoVDA driver was installed but its device did not appear \
                        (a reboot may be required)"
                .into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    log!("SudoVDA driver is ready");
    Ok(())
}

/// `vdisplay uninstall-driver`: removes the SudoVDA device node(s), the driver package
/// and the certificate, elevating if needed.
pub fn uninstall() -> Result<(), String> {
    // Our daemon holds the device open, which would delay removal until reboot.
    if crate::ipc::stop_daemon(15_000)? {
        log!("stopped running virtual display");
    }
    if !is_elevated() {
        return run_elevated("uninstall-driver");
    }
    let removed = with_sudovda_devices(|set, dev| unsafe {
        SetupDiCallClassInstaller(DIF_REMOVE, set, Some(dev))
            .map_err(|e| format!("cannot remove SudoVDA device: {e}"))
    })?;
    log!("removed {removed} SudoVDA device(s)");
    for inf in sudovda_oem_infs() {
        let name = inf.to_string_lossy();
        let ok = unsafe {
            SetupUninstallOEMInfW(PCWSTR(wide(&name).as_ptr()), SUOI_FORCEDELETE, None)
        };
        if ok.as_bool() {
            log!("deleted driver package {name}");
        } else {
            log!(
                "cannot delete driver package {name}: {}",
                windows::core::Error::from_thread()
            );
        }
    }
    remove_certificate(w!("Root"));
    remove_certificate(w!("TrustedPublisher"));
    log!("SudoVDA driver uninstalled");
    Ok(())
}

/// Published names (`oemNN.inf`) of SudoVDA packages in the driver store.
fn sudovda_oem_infs() -> Vec<std::ffi::OsString> {
    let windir = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
    let Ok(entries) = std::fs::read_dir(Path::new(&windir).join("INF")) else {
        return Vec::new();
    };
    let needle = HARDWARE_ID.to_ascii_lowercase();
    entries
        .flatten()
        .map(|e| e.file_name())
        .filter(|name| {
            let n = name.to_string_lossy().to_ascii_lowercase();
            n.starts_with("oem") && n.ends_with(".inf")
        })
        .filter(|name| {
            let Ok(data) = std::fs::read(Path::new(&windir).join("INF").join(name)) else {
                return false;
            };
            // INFs are either UTF-16LE (with BOM) or ANSI.
            let text = if data.starts_with(&[0xFF, 0xFE]) {
                String::from_utf16_lossy(
                    &data[2..]
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect::<Vec<_>>(),
                )
            } else {
                String::from_utf8_lossy(&data).into_owned()
            };
            text.to_ascii_lowercase().contains(&needle)
        })
        .collect()
}

fn install_elevated() -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("vdisplay-sudovda-{}", std::process::id()));
    let result = extract(&dir).and_then(|inf| {
        add_certificate(w!("Root"))?;
        add_certificate(w!("TrustedPublisher"))?;
        if with_sudovda_devices(|_, _| Ok(()))? == 0 {
            create_device_node()?;
            log!("created device node {HARDWARE_ID}");
        }
        install_inf(&inf)
    });
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn extract(dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for (name, data) in FILES {
        std::fs::write(dir.join(name), data)
            .map_err(|e| format!("cannot extract {name}: {e}"))?;
    }
    Ok(dir.join(FILES[0].0))
}

fn open_cert_store(store: PCWSTR) -> Result<HCERTSTORE, String> {
    unsafe {
        CertOpenStore(
            CERT_STORE_PROV_SYSTEM_W,
            CERT_QUERY_ENCODING_TYPE(0),
            None,
            CERT_OPEN_STORE_FLAGS(CERT_SYSTEM_STORE_LOCAL_MACHINE),
            Some(store.as_ptr() as *const c_void),
        )
        .map_err(|e| format!("cannot open certificate store {}: {e}", store.display()))
    }
}

fn remove_certificate(store: PCWSTR) {
    let handle = match open_cert_store(store) {
        Ok(h) => h,
        Err(e) => return log!("{e}"),
    };
    unsafe {
        let cert = CertCreateCertificateContext(X509_ASN_ENCODING, CERT);
        if !cert.is_null() {
            let found = CertFindCertificateInStore(
                handle,
                X509_ASN_ENCODING,
                0,
                CERT_FIND_EXISTING,
                Some(cert as *const c_void),
                None,
            );
            // CertDeleteCertificateFromStore frees `found`, even on failure.
            if !found.is_null() {
                match CertDeleteCertificateFromStore(found) {
                    Ok(()) => log!("removed SudoVDA certificate from {}", store.display()),
                    Err(e) => log!("cannot remove certificate from {}: {e}", store.display()),
                }
            }
            let _ = CertFreeCertificateContext(Some(cert));
        }
        let _ = CertCloseStore(Some(handle), 0);
    }
}

fn add_certificate(store: PCWSTR) -> Result<(), String> {
    let name = unsafe { store.display() };
    let handle = open_cert_store(store)?;
    unsafe {
        let result = CertAddEncodedCertificateToStore(
            Some(handle),
            X509_ASN_ENCODING,
            CERT,
            CERT_STORE_ADD_REPLACE_EXISTING,
            None,
        )
        .map_err(|e| format!("cannot add SudoVDA certificate to {name}: {e}"));
        let _ = CertCloseStore(Some(handle), 0);
        result
    }
}

/// Calls `f` for every present Display-class device with the SudoVDA hardware ID and
/// returns how many there were.
fn with_sudovda_devices(
    mut f: impl FnMut(HDEVINFO, &SP_DEVINFO_DATA) -> Result<(), String>,
) -> Result<usize, String> {
    unsafe {
        let set = SetupDiGetClassDevsW(
            Some(&GUID_DEVCLASS_DISPLAY),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT,
        )
        .map_err(|e| format!("SetupDiGetClassDevs failed: {e}"))?;
        let mut count = 0;
        let mut result = Ok(());
        let mut dev = SP_DEVINFO_DATA {
            cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        let mut index = 0;
        while result.is_ok() && SetupDiEnumDeviceInfo(set, index, &mut dev).is_ok() {
            index += 1;
            let mut buf = [0u8; 1024];
            if SetupDiGetDeviceRegistryPropertyW(
                set,
                &dev,
                SPDRP_HARDWAREID,
                None,
                Some(&mut buf),
                None,
            )
            .is_err()
            {
                continue;
            }
            let ids = String::from_utf16_lossy(
                &buf.chunks_exact(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect::<Vec<_>>(),
            );
            if ids.split('\0').any(|id| id.eq_ignore_ascii_case(HARDWARE_ID)) {
                count += 1;
                result = f(set, &dev);
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
        result.map(|()| count)
    }
}

fn create_device_node() -> Result<(), String> {
    unsafe {
        let set = SetupDiCreateDeviceInfoList(Some(&GUID_DEVCLASS_DISPLAY), None)
            .map_err(|e| format!("SetupDiCreateDeviceInfoList failed: {e}"))?;
        let mut dev = SP_DEVINFO_DATA {
            cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        // REG_MULTI_SZ: the ID followed by an empty string.
        let hwid: Vec<u8> = HARDWARE_ID
            .encode_utf16()
            .chain([0, 0])
            .flat_map(u16::to_le_bytes)
            .collect();
        let result = SetupDiCreateDeviceInfoW(
            set,
            w!("Display"),
            &GUID_DEVCLASS_DISPLAY,
            PCWSTR::null(),
            None,
            DICD_GENERATE_ID,
            Some(&mut dev),
        )
        .and_then(|()| SetupDiSetDeviceRegistryPropertyW(set, &mut dev, SPDRP_HARDWAREID, Some(&hwid)))
        .and_then(|()| SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set, Some(&dev)))
        .map_err(|e| format!("cannot create SudoVDA device node: {e}"));
        let _ = SetupDiDestroyDeviceInfoList(set);
        result
    }
}

fn install_inf(inf: &Path) -> Result<(), String> {
    let hwid = wide(HARDWARE_ID);
    let path = wide(&inf.to_string_lossy());
    let mut reboot = BOOL(0);
    unsafe {
        UpdateDriverForPlugAndPlayDevicesW(
            None,
            PCWSTR(hwid.as_ptr()),
            PCWSTR(path.as_ptr()),
            INSTALLFLAG_FORCE,
            Some(&mut reboot),
        )
    }
    .map_err(|e| format!("cannot install SudoVDA driver: {e}"))?;
    log!(
        "SudoVDA driver installed{}",
        if reboot.as_bool() { " (reboot required)" } else { "" }
    );
    Ok(())
}

fn is_elevated() -> bool {
    unsafe { IsUserAnAdmin().as_bool() }
}

/// Re-runs this executable elevated (UAC prompt) with `command` and waits for it.
fn run_elevated(command: &str) -> Result<(), String> {
    log!("requesting administrator rights to run `{command}`");
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = wide(&exe.to_string_lossy());
    let params = wide(command);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(exe.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        ..Default::default()
    };
    unsafe {
        ShellExecuteExW(&mut info).map_err(|e| format!("cannot elevate `{command}`: {e}"))?;
        if info.hProcess.is_invalid() {
            return Err(format!("cannot elevate `{command}`"));
        }
        let waited = WaitForSingleObject(info.hProcess, 5 * 60 * 1000);
        let mut code = 1u32;
        let _ = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        if waited != WAIT_OBJECT_0 {
            return Err(format!("timed out waiting for elevated `{command}`"));
        }
        if code != 0 {
            return Err(format!(
                "elevated `{command}` failed (see {})",
                crate::logging::path().display()
            ));
        }
    }
    Ok(())
}

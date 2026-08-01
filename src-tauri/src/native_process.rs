//! Minimal read-only process transport for the future native telemetry reader.
//!
//! This module deliberately exposes no write, allocation, thread or hook API.

use serde::Serialize;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ProcessReadError {
    InvalidProcessId,
    OpenFailed,
    ModuleUnavailable,
    AddressOverflow,
    ReadFailed,
    PartialRead,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NativeTransportState {
    Missing,
    Ready,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeTransportProbe {
    state: NativeTransportState,
    module_size: Option<usize>,
    issue: Option<ProcessReadError>,
}

pub(crate) fn probe_native_process(process_id: u32) -> NativeTransportProbe {
    if process_id == 0 {
        return NativeTransportProbe {
            state: NativeTransportState::Missing,
            module_size: None,
            issue: Some(ProcessReadError::InvalidProcessId),
        };
    }
    let reader = match NativeProcessReader::open(process_id) {
        Ok(reader) => reader,
        Err(issue) => {
            return NativeTransportProbe {
                state: NativeTransportState::Unavailable,
                module_size: None,
                issue: Some(issue),
            }
        }
    };
    match validate_pe64_image(&reader) {
        Ok(()) => NativeTransportProbe {
            state: NativeTransportState::Ready,
            module_size: Some(reader.module_size()),
            issue: None,
        },
        Err(issue) => NativeTransportProbe {
            state: NativeTransportState::Unavailable,
            module_size: None,
            issue: Some(issue),
        },
    }
}

pub(crate) fn game_log_directory(process_id: u32) -> Option<PathBuf> {
    platform::main_module_path(process_id)
        .ok()
        .and_then(|path| path.parent()?.parent().map(|root| root.join("Logs")))
}

fn validate_pe64_image(reader: &NativeProcessReader) -> Result<(), ProcessReadError> {
    let mut dos = [0_u8; 64];
    reader.read_module_exact(0, &mut dos)?;
    if &dos[..2] != b"MZ" {
        return Err(ProcessReadError::ReadFailed);
    }
    let pe_offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as usize;
    let mut header = [0_u8; 6];
    reader.read_module_exact(pe_offset, &mut header)?;
    if &header[..4] != b"PE\0\0" || u16::from_le_bytes([header[4], header[5]]) != 0x8664 {
        return Err(ProcessReadError::ReadFailed);
    }
    Ok(())
}

#[cfg(target_os = "windows")]
mod platform {
    use std::{ffi::c_void, mem::size_of, path::PathBuf};

    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Diagnostics::{
                Debug::ReadProcessMemory,
                ToolHelp::{
                    CreateToolhelp32Snapshot, Module32FirstW, MODULEENTRY32W, TH32CS_SNAPMODULE,
                    TH32CS_SNAPMODULE32,
                },
            },
            Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ},
        },
    };

    use super::ProcessReadError;

    pub(crate) struct NativeProcessReader {
        process: HANDLE,
        module_base: usize,
        module_size: usize,
    }

    impl NativeProcessReader {
        pub(crate) fn open(process_id: u32) -> Result<Self, ProcessReadError> {
            if process_id == 0 {
                return Err(ProcessReadError::InvalidProcessId);
            }
            let process = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
                    false,
                    process_id,
                )
            }
            .map_err(|_| ProcessReadError::OpenFailed)?;

            match main_module(process_id) {
                Ok((module_base, module_size)) => Ok(Self {
                    process,
                    module_base,
                    module_size,
                }),
                Err(error) => {
                    unsafe {
                        let _ = CloseHandle(process);
                    }
                    Err(error)
                }
            }
        }

        pub(crate) const fn module_size(&self) -> usize {
            self.module_size
        }

        pub(crate) fn read_module_exact(
            &self,
            relative_offset: usize,
            output: &mut [u8],
        ) -> Result<(), ProcessReadError> {
            let end = relative_offset
                .checked_add(output.len())
                .ok_or(ProcessReadError::AddressOverflow)?;
            if end > self.module_size {
                return Err(ProcessReadError::AddressOverflow);
            }
            let address = self
                .module_base
                .checked_add(relative_offset)
                .ok_or(ProcessReadError::AddressOverflow)?;
            let mut bytes_read = 0_usize;
            unsafe {
                ReadProcessMemory(
                    self.process,
                    address as *const c_void,
                    output.as_mut_ptr().cast(),
                    output.len(),
                    Some(&mut bytes_read),
                )
            }
            .map_err(|_| ProcessReadError::ReadFailed)?;
            if bytes_read != output.len() {
                return Err(ProcessReadError::PartialRead);
            }
            Ok(())
        }
    }

    impl Drop for NativeProcessReader {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.process);
            }
        }
    }

    pub(super) fn main_module_path(process_id: u32) -> Result<PathBuf, ProcessReadError> {
        let snapshot = unsafe {
            CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, process_id)
        }
        .map_err(|_| ProcessReadError::ModuleUnavailable)?;
        let mut entry = MODULEENTRY32W {
            dwSize: size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        let result = unsafe { Module32FirstW(snapshot, &mut entry) };
        unsafe {
            let _ = CloseHandle(snapshot);
        }
        result.map_err(|_| ProcessReadError::ModuleUnavailable)?;
        let end = entry
            .szExePath
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(entry.szExePath.len());
        Ok(PathBuf::from(String::from_utf16_lossy(
            &entry.szExePath[..end],
        )))
    }

    fn main_module(process_id: u32) -> Result<(usize, usize), ProcessReadError> {
        let snapshot = unsafe {
            CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, process_id)
        }
        .map_err(|_| ProcessReadError::ModuleUnavailable)?;
        let mut entry = MODULEENTRY32W {
            dwSize: size_of::<MODULEENTRY32W>() as u32,
            ..Default::default()
        };
        let result = unsafe { Module32FirstW(snapshot, &mut entry) };
        unsafe {
            let _ = CloseHandle(snapshot);
        }
        result.map_err(|_| ProcessReadError::ModuleUnavailable)?;
        let base = entry.modBaseAddr as usize;
        let size = entry.modBaseSize as usize;
        if base == 0 || size == 0 {
            return Err(ProcessReadError::ModuleUnavailable);
        }
        Ok((base, size))
    }
}

#[cfg(target_os = "windows")]
pub(crate) use platform::NativeProcessReader;

#[cfg(not(target_os = "windows"))]
struct NativeProcessReader;

#[cfg(not(target_os = "windows"))]
impl NativeProcessReader {
    fn open(_process_id: u32) -> Result<Self, ProcessReadError> {
        Err(ProcessReadError::OpenFailed)
    }

    const fn module_size(&self) -> usize {
        0
    }

    fn read_module_exact(
        &self,
        _relative_offset: usize,
        _output: &mut [u8],
    ) -> Result<(), ProcessReadError> {
        Err(ProcessReadError::ReadFailed)
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use super::*;
    pub(super) fn main_module_path(_process_id: u32) -> Result<PathBuf, ProcessReadError> {
        Err(ProcessReadError::ModuleUnavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_pid_is_rejected() {
        assert_eq!(probe_native_process(0).state, NativeTransportState::Missing);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn current_process_image_is_read_without_write_access() {
        let probe = probe_native_process(std::process::id());
        assert_eq!(probe.state, NativeTransportState::Ready);
        assert!(probe.module_size.is_some_and(|size| size > 64 * 1024));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn read_is_bounded_to_the_main_module() {
        let reader = NativeProcessReader::open(std::process::id()).unwrap();
        let mut byte = [0_u8; 1];
        assert_eq!(
            reader.read_module_exact(reader.module_size(), &mut byte),
            Err(ProcessReadError::AddressOverflow)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires an explicitly selected running Scrap Mechanic PID"]
    fn live_game_process_transport_is_ready() {
        let process_id: u32 = std::env::var("SCRAPMAP_LIVE_GAME_PID")
            .expect("SCRAPMAP_LIVE_GAME_PID is required")
            .parse()
            .expect("SCRAPMAP_LIVE_GAME_PID must be a u32");
        let probe = probe_native_process(process_id);
        assert_eq!(probe.state, NativeTransportState::Ready);
        assert!(probe.module_size.is_some_and(|size| size > 1024 * 1024));
        assert_eq!(probe.issue, None);
    }
}

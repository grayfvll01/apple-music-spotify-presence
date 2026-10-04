//! "Start with Windows" via the per-user Run key (no admin needed).

use crate::prelude::*;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_BINARY, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
    RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

const RUN: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
/// Where Task Manager / Settings > Startup apps record "disabled" for a Run
/// entry (first byte odd = disabled) without deleting it.
const APPROVED: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");
const NAME: PCWSTR = w!("AppleMusicSpotifyPresence");
/// The Run entry's names before the app was renamed (1.1.x, 0.1.x).
const OLD_NAMES: [PCWSTR; 2] = [w!("AppleMusicDiscordPresence"), w!("ap-music-drp")];

fn command() -> Option<String> {
    let exe = crate::sys::exe_path();
    (!exe.is_empty()).then(|| format!("\"{exe}\""))
}

/// True if the Run entry exists, points at this exe, and wasn't switched off
/// in Task Manager.
pub fn enabled() -> bool {
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let ok = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, RUN, NAME, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len))
    }
    .is_ok();
    let got = String::from_utf16_lossy(&buf[..(len as usize / 2).min(buf.len())]);
    let ours = ok && command().is_some_and(|want| got.trim_end_matches('\0').eq_ignore_ascii_case(&want));

    let mut flag = [0u8; 12];
    let mut flen = flag.len() as u32;
    let found = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            APPROVED,
            NAME,
            RRF_RT_REG_BINARY,
            None,
            Some(flag.as_mut_ptr().cast()),
            Some(&mut flen),
        )
    }
    .is_ok();
    ours && !(found && flen > 0 && flag[0] & 1 == 1)
}

/// Moves a "Start with Windows" entry from before a rename to the current
/// name and exe, still switched off if it was off in Task Manager.
pub fn migrate() {
    for old in OLD_NAMES {
        unsafe {
            if RegGetValueW(HKEY_CURRENT_USER, RUN, old, RRF_RT_REG_SZ, None, None, None).is_err() {
                continue;
            }
            let mut flag = [0u8; 12];
            let mut flen = flag.len() as u32;
            let flagged = RegGetValueW(
                HKEY_CURRENT_USER,
                APPROVED,
                old,
                RRF_RT_REG_BINARY,
                None,
                Some(flag.as_mut_ptr().cast()),
                Some(&mut flen),
            )
            .is_ok();
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, old);
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, APPROVED, old);
            set(true);
            if flagged {
                let _ =
                    RegSetKeyValueW(HKEY_CURRENT_USER, APPROVED, NAME, REG_BINARY.0, Some(flag.as_ptr().cast()), flen);
            }
        }
    }
}

pub fn set(on: bool) {
    unsafe {
        // Clearing the Task Manager flag makes the Run entry count again.
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, APPROVED, NAME);
        if on {
            if let Some(cmd) = command() {
                let cmd: Vec<u16> = cmd.encode_utf16().chain([0]).collect();
                let _ = RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    RUN,
                    NAME,
                    REG_SZ.0,
                    Some(cmd.as_ptr().cast()),
                    (cmd.len() * 2) as u32,
                );
            }
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME);
        }
    }
}

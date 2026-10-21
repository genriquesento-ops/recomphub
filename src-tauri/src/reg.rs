//! Small Windows registry helpers (no administrator needed for what RecompHUB reads and writes).

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ,
    RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
};

pub enum Root {
    User,
    Machine,
}

fn hkey(root: Root) -> HKEY {
    match root {
        Root::User => HKEY_CURRENT_USER,
        Root::Machine => HKEY_LOCAL_MACHINE,
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn get_string(root: Root, key: &str, value: &str) -> Option<String> {
    let (k, v) = (wide(key), wide(value));
    let mut buf = vec![0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: the buffer and its size in bytes go together; the names end in 0.
    let r = unsafe {
        RegGetValueW(hkey(root), k.as_ptr(), v.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
    };
    if r != ERROR_SUCCESS {
        return None;
    }
    let len = (size as usize / 2).min(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]).trim_end_matches('\0').to_string())
}

pub fn get_dword(root: Root, key: &str, value: &str) -> Option<u32> {
    let (k, v) = (wide(key), wide(value));
    let mut out: u32 = 0;
    let mut size = 4u32;
    // SAFETY: a 4-byte output for a DWORD; the names end in 0.
    let r = unsafe {
        RegGetValueW(hkey(root), k.as_ptr(), v.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut out as *mut u32).cast(), &mut size)
    };
    (r == ERROR_SUCCESS).then_some(out)
}

/// Writes a text value under HKEY_CURRENT_USER, creating the key when needed.
pub fn set_user_string(key: &str, value: &str, data: &str) -> Result<(), String> {
    let (k, v, d) = (wide(key), wide(value), wide(data));
    // SAFETY: the data is a NUL-terminated UTF-16 string and its size includes the NUL.
    let r = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), REG_SZ, d.as_ptr().cast(), (d.len() * 2) as u32) };
    if r == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!("O Windows não deixou gravar a configuração (erro {r})."))
    }
}

/// Removes a value under HKEY_CURRENT_USER; a value that is not there is fine.
pub fn delete_user_value(key: &str, value: &str) -> Result<(), String> {
    const ERROR_FILE_NOT_FOUND: u32 = 2;
    let (k, v) = (wide(key), wide(value));
    // SAFETY: both names end in 0.
    let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr()) };
    if r == ERROR_SUCCESS || r == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!("O Windows não deixou apagar a configuração (erro {r})."))
    }
}

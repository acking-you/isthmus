//! Windows: the clipboard's sequence number moves with every copy and is
//! read without opening the clipboard; the text itself is `CF_UNICODETEXT`,
//! read and written with the clipboard open, which another program may
//! hold for a moment, so opening is tried a few times.

use std::time::Duration;

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};

use crate::{Access, Contents, Error, MAX_TEXT};

pub(crate) const SUPPORTED: bool = true;

/// `CF_UNICODETEXT`, which windows-sys keeps in a module this crate has no
/// other use for.
const CF_UNICODETEXT: u32 = 13;

/// The format careful apps put beside what must not be recorded or synced
/// (passwords), which clipboard managers honour.
const DO_NOT_SHARE: &str = "ExcludeClipboardContentFromMonitorProcessing";

pub(crate) fn change_count() -> Option<u64> {
    // SAFETY: no arguments; reads a counter.
    Some(u64::from(unsafe { GetClipboardSequenceNumber() }))
}

/// Windows lets any program read the clipboard.
pub(crate) fn access() -> Access {
    Access::Allowed
}

/// The clipboard, open until dropped.
struct Open;

impl Open {
    fn new() -> Result<Self, Error> {
        for _ in 0..10 {
            // SAFETY: a null owner opens it for this task.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(Self);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(Error(format!(
            "another program holds the clipboard: {}",
            std::io::Error::last_os_error()
        )))
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: opened in `new`, closed once.
        unsafe { CloseClipboard() };
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub(crate) fn read() -> Result<Contents, Error> {
    let _open = Open::new()?;
    let marker = wide(DO_NOT_SHARE);
    // SAFETY: a NUL-terminated name; registering returns its format id.
    let format = unsafe { RegisterClipboardFormatW(marker.as_ptr()) };
    // SAFETY: the clipboard is open.
    if format != 0 && unsafe { IsClipboardFormatAvailable(format) } != 0 {
        return Ok(Contents::Concealed);
    }
    // SAFETY: the clipboard is open.
    let handle = unsafe { GetClipboardData(CF_UNICODETEXT) };
    if handle.is_null() {
        return Ok(Contents::Other);
    }
    // SAFETY: a clipboard handle to global memory, valid while it is open.
    let size = unsafe { GlobalSize(handle) } / 2;
    if size == 0 || size > MAX_TEXT {
        return Ok(Contents::Other);
    }
    // SAFETY: as above; locked for the copy below.
    let data = unsafe { GlobalLock(handle) }.cast::<u16>();
    if data.is_null() {
        return Ok(Contents::Other);
    }
    // SAFETY: `size` u16s of the locked block are readable.
    let units = unsafe { std::slice::from_raw_parts(data, size) };
    let end = units
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(units.len());
    let text = String::from_utf16_lossy(units.get(..end).unwrap_or_default());
    // SAFETY: locked above.
    unsafe { GlobalUnlock(handle) };
    Ok(Contents::Text(text))
}

pub(crate) fn write(text: &str) -> Result<(), Error> {
    let units = wide(text);
    let bytes = units.len() * 2;
    let _open = Open::new()?;
    // SAFETY: the clipboard is open.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(Error(format!(
            "the clipboard could not be emptied: {}",
            std::io::Error::last_os_error()
        )));
    }
    // SAFETY: a fresh movable block of `bytes`.
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes) };
    if memory.is_null() {
        return Err(Error("no memory for the clipboard".into()));
    }
    // SAFETY: the block just allocated, locked for the copy.
    let target = unsafe { GlobalLock(memory) }.cast::<u16>();
    if target.is_null() {
        // SAFETY: still ours.
        unsafe { GlobalFree(memory) };
        return Err(Error("no memory for the clipboard".into()));
    }
    // SAFETY: `units.len()` u16s fit the block, which nothing else uses.
    unsafe {
        std::ptr::copy_nonoverlapping(units.as_ptr(), target, units.len());
        GlobalUnlock(memory);
    }
    // SAFETY: the clipboard takes the block on success.
    let placed: HANDLE = unsafe { SetClipboardData(CF_UNICODETEXT, memory) };
    if placed.is_null() {
        // SAFETY: not taken, so still ours.
        unsafe { GlobalFree(memory) };
        return Err(Error(format!(
            "Windows did not take the text: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(())
}

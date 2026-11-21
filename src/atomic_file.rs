use std::{io, path::Path};

pub(super) fn install(temporary: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    if !destination.try_exists()? {
        return std::fs::rename(temporary, destination);
    }
    replace(temporary, destination)
}

#[cfg(not(windows))]
pub(super) use std::fs::rename as replace;

#[cfg(windows)]
pub(super) fn replace(temporary: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>()
    };
    let destination = wide(destination);
    let replacement = wide(temporary);
    let result = unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

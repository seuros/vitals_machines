use std::ffi::CStr;

pub fn integer(name: &CStr) -> Option<i32> {
    let mut value = 0_i32;
    let mut size = size_of::<i32>();
    // SAFETY: both output pointers are valid for the declared sizes; read-only.
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&raw mut value).cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0 && size == size_of::<i32>()).then_some(value)
}

pub fn string(name: &CStr) -> Option<String> {
    // Model/guest strings are small. Reject truncation rather than using a
    // partial identifier as classification evidence.
    let mut bytes = [0_u8; 1024];
    let mut size = bytes.len();
    // SAFETY: the output buffer and its length are valid; read-only query.
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            bytes.as_mut_ptr().cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return None;
    }
    let value = CStr::from_bytes_until_nul(bytes.get(..size)?).ok()?;
    let value = value.to_str().ok()?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// Enumerate only one subtree through FreeBSD's native sysctl OID iterator.
/// Failure, malformed replies, or hitting the budget is not a complete tree.
#[cfg(target_os = "freebsd")]
pub fn names(root: &CStr, budget: usize) -> Option<Vec<std::ffi::CString>> {
    const MAX_NAME: usize = 24; // CTL_MAXNAME
    let mut prefix = [0_i32; MAX_NAME];
    let mut prefix_len = prefix.len();
    // SAFETY: prefix and prefix_len are writable for MAX_NAME entries; root is terminated.
    if unsafe { libc::sysctlnametomib(root.as_ptr(), prefix.as_mut_ptr(), &raw mut prefix_len) }
        != 0
    {
        return None;
    }
    let prefix = prefix
        .get(..prefix_len)
        .filter(|prefix| !prefix.is_empty())?;
    let mut cursor = prefix.to_vec();
    let mut names = Vec::new();
    for _ in 0..budget {
        let mut next = [0_i32; MAX_NAME];
        // CTL_SYSCTL, CTL_SYSCTL_NEXTNOSKIP
        let size = match oid_query(&[&[0, 7][..], &cursor].concat(), &mut next) {
            Ok(size) => size,
            Err(error) => return (error.raw_os_error() == Some(libc::ENOENT)).then_some(names),
        };
        if size == 0 || size % size_of::<i32>() != 0 {
            return None;
        }
        let next = next.get(..size / size_of::<i32>())?;
        if !next.starts_with(prefix) {
            return Some(names);
        }
        if next <= cursor.as_slice() {
            return None;
        }
        let mut bytes = [0_u8; 1024];
        // CTL_SYSCTL, CTL_SYSCTL_NAME: returns the bounded name, never a value.
        let size = oid_query(&[&[0, 1][..], next].concat(), &mut bytes).ok()?;
        let name = CStr::from_bytes_with_nul(bytes.get(..size)?).ok()?;
        let suffix = name.to_bytes().strip_prefix(root.to_bytes())?;
        if !suffix.starts_with(b".") {
            return None;
        }
        names.push(name.to_owned());
        cursor = next.to_vec();
    }
    None
}

/// Read-only MIB query into `out`; the reply size in bytes, never above its capacity.
#[cfg(target_os = "freebsd")]
fn oid_query<T: Copy>(mib: &[i32], out: &mut [T]) -> std::io::Result<usize> {
    use std::io::{Error, ErrorKind};

    let len = u32::try_from(mib.len()).map_err(|_| Error::from(ErrorKind::InvalidInput))?;
    let mut size = size_of_val(out);
    // SAFETY: mib is a readable OID of len entries; out/size describe one writable
    // buffer of exactly size bytes. No new value is supplied.
    let result = unsafe {
        libc::sysctl(
            mib.as_ptr(),
            len,
            out.as_mut_ptr().cast(),
            &raw mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        Err(Error::last_os_error())
    } else if size > size_of_val(out) {
        Err(Error::from(ErrorKind::InvalidData))
    } else {
        Ok(size)
    }
}

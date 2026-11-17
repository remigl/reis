//! Varints, little-endian helpers and a read-only memory-mapped blob.

pub fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

#[inline]
pub fn get_varint(b: &[u8], p: &mut usize) -> u64 {
    let mut v = 0u64;
    let mut s = 0;
    loop {
        let c = b[*p];
        *p += 1;
        v |= ((c & 0x7F) as u64) << s;
        if c < 0x80 {
            return v;
        }
        s += 7;
    }
}

#[inline]
pub fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
#[inline]
pub fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

/// A file mapped read-only (unix: mmap; elsewhere: read into memory).
pub struct Blob {
    #[cfg(unix)]
    ptr: *mut u8,
    #[cfg(unix)]
    len: usize,
    #[cfg(not(unix))]
    vec: Vec<u8>,
}

unsafe impl Send for Blob {}
unsafe impl Sync for Blob {}

#[cfg(unix)]
mod sys {
    extern "C" {
        pub fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64)
            -> *mut u8;
        pub fn munmap(addr: *mut u8, len: usize) -> i32;
    }
    pub const PROT_READ: i32 = 1;
    pub const MAP_PRIVATE: i32 = 2;
}

impl Blob {
    pub fn open(path: &str) -> std::io::Result<Blob> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let f = std::fs::File::open(path)?;
            let len = f.metadata()?.len() as usize;
            if len == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "empty file",
                ));
            }
            let ptr = unsafe {
                sys::mmap(
                    std::ptr::null_mut(),
                    len,
                    sys::PROT_READ,
                    sys::MAP_PRIVATE,
                    f.as_raw_fd(),
                    0,
                )
            };
            if ptr as isize == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Blob { ptr, len })
        }
        #[cfg(not(unix))]
        {
            Ok(Blob {
                vec: std::fs::read(path)?,
            })
        }
    }
    pub fn bytes(&self) -> &[u8] {
        #[cfg(unix)]
        unsafe {
            std::slice::from_raw_parts(self.ptr, self.len)
        }
        #[cfg(not(unix))]
        {
            &self.vec
        }
    }
}

impl Drop for Blob {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            sys::munmap(self.ptr, self.len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn varint_roundtrip() {
        let mut v = vec![];
        for x in [0u64, 1, 127, 128, 300, 1 << 40] {
            put_varint(&mut v, x);
        }
        let mut p = 0;
        for x in [0u64, 1, 127, 128, 300, 1 << 40] {
            assert_eq!(get_varint(&v, &mut p), x);
        }
    }
}

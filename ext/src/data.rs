use memmap2::Mmap;
use std::fs::File;
use std::io::{self, Read};

// 14 dims úteis + 2 zeros = 16 i16 = 32 bytes (1 _mm256_loadu_si256).
pub const VEC_DIMS_PADDED: usize = 16;
pub const VEC_BYTES: usize = VEC_DIMS_PADDED * 2;

pub struct Vectors {
    pub mmap: Mmap,
    pub count: u32,
    pub payload_offset: usize,
}

pub fn load_vectors(path: &str) -> io::Result<Vectors> {
    let file = File::open(path)?;
    let mut header = [0u8; 16];
    file.try_clone()?.read_exact(&mut header)?;
    if &header[0..4] != b"VEC4" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "magic invalido"));
    }
    let version = u32::from_le_bytes(header[4..8].try_into().unwrap());
    let count = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let dims = u32::from_le_bytes(header[12..16].try_into().unwrap());
    if version != 1 || dims != 14 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "version/dims"));
    }
    let mmap = unsafe { Mmap::map(&file)? };
    let _ = mmap.advise(memmap2::Advice::WillNeed);
    let _ = mmap.lock();
    Ok(Vectors { mmap, count, payload_offset: 16 })
}

pub fn load_labels(path: &str, count: u32) -> io::Result<Mmap> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    if len != count as u64 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "labels size"));
    }
    let m = unsafe { Mmap::map(&file)? };
    let _ = m.advise(memmap2::Advice::WillNeed);
    let _ = m.lock();
    Ok(m)
}

#[inline]
pub fn vec_ptr(payload: &[u8], idx: usize) -> *const i16 {
    unsafe { payload.as_ptr().add(idx * VEC_BYTES) as *const i16 }
}

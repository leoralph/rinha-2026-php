use crate::data::{vec_ptr, VEC_BYTES};
use std::io::{self, Read};

#[derive(Clone, Copy)]
pub struct Node {
    pub threshold_sq: i64,
    pub range_lo: u32,
    pub range_hi: u32,
    pub right_child_idx: i32,
}

pub fn load_vptree(path: &str, expected_count: u32) -> io::Result<(Vec<Node>, u32)> {
    let mut file = std::fs::File::open(path)?;
    let mut header = [0u8; 16];
    file.read_exact(&mut header)?;
    if &header[0..4] != b"VPT2" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "magic invalido"));
    }
    let version = u32::from_le_bytes(header[4..8].try_into().unwrap());
    let bucket = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let total = u32::from_le_bytes(header[12..16].try_into().unwrap());
    if version != 1 || total != expected_count {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "version/count"));
    }
    let mut body = Vec::new();
    file.read_to_end(&mut body)?;
    if body.len() % 20 != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "body size"));
    }
    let n = body.len() / 20;
    let mut nodes = Vec::with_capacity(n);
    for i in 0..n {
        let off = i * 20;
        nodes.push(Node {
            threshold_sq: i64::from_le_bytes(body[off..off + 8].try_into().unwrap()),
            range_lo: u32::from_le_bytes(body[off + 8..off + 12].try_into().unwrap()),
            range_hi: u32::from_le_bytes(body[off + 12..off + 16].try_into().unwrap()),
            right_child_idx: i32::from_le_bytes(body[off + 16..off + 20].try_into().unwrap()),
        });
    }
    if nodes.is_empty() || nodes[0].range_lo != 0 || nodes[0].range_hi != total {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "raiz mal-formada"));
    }
    Ok((nodes, bucket))
}

// L2² em i64 (max 14×20000² = 5.6B ultrapassa i32). Vetores padded a 16 i16
// pra um único _mm256_loadu_si256.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub unsafe fn dist_sq(query: *const i16, vector: *const i16) -> i64 {
    use std::arch::x86_64::*;
    let q = _mm256_loadu_si256(query as *const __m256i);
    let v = _mm256_loadu_si256(vector as *const __m256i);
    let diff = _mm256_sub_epi16(v, q);
    let sq = _mm256_madd_epi16(diff, diff);
    let zero = _mm256_setzero_si256();
    let lo = _mm256_unpacklo_epi32(sq, zero);
    let hi = _mm256_unpackhi_epi32(sq, zero);
    let total = _mm256_add_epi64(lo, hi);
    let lo128 = _mm256_castsi256_si128(total);
    let hi128 = _mm256_extracti128_si256(total, 1);
    let s2 = _mm_add_epi64(lo128, hi128);
    let r = _mm_add_epi64(s2, _mm_unpackhi_epi64(s2, s2));
    _mm_cvtsi128_si64(r) as i64
}

#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
pub unsafe fn dist_sq(query: *const i16, vector: *const i16) -> i64 {
    let mut s: i64 = 0;
    for i in 0..14 {
        let d = (*vector.add(i)) as i64 - (*query.add(i)) as i64;
        s += d * d;
    }
    s
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn prefetch_t0(p: *const u8) {
    unsafe { std::arch::x86_64::_mm_prefetch(p as *const i8, std::arch::x86_64::_MM_HINT_T0); }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
fn prefetch_t0(_p: *const u8) {}

pub const KNN: usize = 5;

#[derive(Clone, Copy)]
pub struct Result {
    pub distance: i64,
    pub index: u32,
}

#[inline(always)]
fn push_heap(heap: &mut Vec<Result>, d: i64, idx: u32) {
    if heap.len() < KNN {
        heap.push(Result { distance: d, index: idx });
        let mut i = heap.len() - 1;
        while i > 0 {
            let parent = (i - 1) / 2;
            if heap[parent].distance >= heap[i].distance { break; }
            heap.swap(parent, i);
            i = parent;
        }
    } else if d < heap[0].distance {
        heap[0] = Result { distance: d, index: idx };
        let mut i = 0;
        let n = heap.len();
        loop {
            let l = 2 * i + 1;
            let r = 2 * i + 2;
            let mut largest = i;
            if l < n && heap[l].distance > heap[largest].distance { largest = l; }
            if r < n && heap[r].distance > heap[largest].distance { largest = r; }
            if largest == i { break; }
            heap.swap(i, largest);
            i = largest;
        }
    }
}

pub fn search_vptree(nodes: &[Node], vectors: &[u8], query: *const i16) -> Vec<Result> {
    let mut heap: Vec<Result> = Vec::with_capacity(KNN);
    search_node(nodes, vectors, query, &mut heap, 0);
    heap
}

pub fn search_into(nodes: &[Node], vectors: &[u8], query: *const i16, heap: &mut Vec<Result>) {
    search_node(nodes, vectors, query, heap, 0);
}

fn search_node(nodes: &[Node], vectors: &[u8], query: *const i16, heap: &mut Vec<Result>, idx: usize) {
    let n = nodes[idx];

    if n.right_child_idx == -1 {
        let lo = n.range_lo as usize;
        let hi = n.range_hi as usize;
        prefetch_t0(unsafe { vectors.as_ptr().add(lo * VEC_BYTES) });
        for i in lo..hi {
            if i + 1 < hi {
                prefetch_t0(unsafe { vectors.as_ptr().add((i + 1) * VEC_BYTES) });
            }
            let d = unsafe { dist_sq(query, vec_ptr(vectors, i)) };
            push_heap(heap, d, i as u32);
        }
        return;
    }

    let pivot_idx = n.range_lo as usize;
    let d = unsafe { dist_sq(query, vec_ptr(vectors, pivot_idx)) };
    push_heap(heap, d, pivot_idx as u32);

    let left_idx = idx + 1;
    let right_idx = n.right_child_idx as usize;
    let threshold = n.threshold_sq;

    let (near_idx, far_idx) = if d < threshold { (left_idx, right_idx) } else { (right_idx, left_idx) };
    search_node(nodes, vectors, query, heap, near_idx);

    let worst_sq = if heap.len() >= KNN { heap[0].distance } else { i64::MAX };
    let diff = (d as f64).sqrt() - (threshold as f64).sqrt();
    if diff * diff < worst_sq as f64 {
        search_node(nodes, vectors, query, heap, far_idx);
    }
}

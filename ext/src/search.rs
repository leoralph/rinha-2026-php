use crate::data::{read_dim, VEC_BYTES};
use std::io::{self, Read};

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn prefetch_t0(p: *const u8) {
    unsafe { std::arch::x86_64::_mm_prefetch(p as *const i8, std::arch::x86_64::_MM_HINT_T0); }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
fn prefetch_t0(_p: *const u8) {}

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

#[inline(always)]
fn dist_sq(query: &[i32; 14], vectors: &[u8], base: usize) -> i64 {
    let d0 = read_dim(vectors, base) - query[0];
    let d1 = read_dim(vectors, base + 3) - query[1];
    let d2 = read_dim(vectors, base + 6) - query[2];
    let d3 = read_dim(vectors, base + 9) - query[3];
    let d4 = read_dim(vectors, base + 12) - query[4];
    let d5 = read_dim(vectors, base + 15) - query[5];
    let d6 = read_dim(vectors, base + 18) - query[6];
    let d7 = read_dim(vectors, base + 21) - query[7];
    let d8 = read_dim(vectors, base + 24) - query[8];
    let d9 = read_dim(vectors, base + 27) - query[9];
    let d10 = read_dim(vectors, base + 30) - query[10];
    let d11 = read_dim(vectors, base + 33) - query[11];
    let d12 = read_dim(vectors, base + 36) - query[12];
    let d13 = read_dim(vectors, base + 39) - query[13];
    (d0 as i64).pow(2) + (d1 as i64).pow(2) + (d2 as i64).pow(2)
        + (d3 as i64).pow(2) + (d4 as i64).pow(2) + (d5 as i64).pow(2)
        + (d6 as i64).pow(2) + (d7 as i64).pow(2) + (d8 as i64).pow(2)
        + (d9 as i64).pow(2) + (d10 as i64).pow(2) + (d11 as i64).pow(2)
        + (d12 as i64).pow(2) + (d13 as i64).pow(2)
}

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
            if heap[parent].distance >= heap[i].distance {
                break;
            }
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
            if l < n && heap[l].distance > heap[largest].distance {
                largest = l;
            }
            if r < n && heap[r].distance > heap[largest].distance {
                largest = r;
            }
            if largest == i {
                break;
            }
            heap.swap(i, largest);
            i = largest;
        }
    }
}

pub fn search_vptree(nodes: &[Node], vectors: &[u8], query: &[i32; 14]) -> Vec<Result> {
    let mut heap: Vec<Result> = Vec::with_capacity(KNN);
    search_node(nodes, vectors, query, &mut heap, 0);
    heap
}

fn search_node(nodes: &[Node], vectors: &[u8], query: &[i32; 14], heap: &mut Vec<Result>, idx: usize) {
    let n = nodes[idx];

    if n.right_child_idx == -1 {
        // Prefetch primeira linha da leaf (geralmente cold no L1).
        let leaf_base = (n.range_lo as usize) * VEC_BYTES;
        prefetch_t0(unsafe { vectors.as_ptr().add(leaf_base) });
        for i in n.range_lo..n.range_hi {
            let off = (i as usize) * VEC_BYTES;
            // Prefetch da próxima linha enquanto processamos a atual.
            if i + 1 < n.range_hi {
                prefetch_t0(unsafe { vectors.as_ptr().add(off + VEC_BYTES) });
            }
            let d = dist_sq(query, vectors, off);
            push_heap(heap, d, i);
        }
        return;
    }

    let pivot_idx = n.range_lo;
    let d = dist_sq(query, vectors, (pivot_idx as usize) * VEC_BYTES);
    push_heap(heap, d, pivot_idx);

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

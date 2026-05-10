mod data;
mod search;
mod vector;

use ext_php_rs::prelude::*;
use mimalloc::MiMalloc;
use memmap2::Mmap;
use once_cell::sync::OnceCell;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const NORMALIZATION_JSON: &str = include_str!("../resources/normalization.json");
const MCC_RISK_JSON: &str = include_str!("../resources/mcc_risk.json");

const READY_RESPONSE: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n";
const NOT_FOUND: &[u8] =
    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n";
const BAD_REQUEST: &[u8] =
    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const ERROR_500: &[u8] =
    b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

const FRAUD_RESPONSES: [&[u8]; 6] = [
    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 33\r\nConnection: keep-alive\r\n\r\n{\"approved\":true,\"fraud_score\":0}",
    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 35\r\nConnection: keep-alive\r\n\r\n{\"approved\":true,\"fraud_score\":0.2}",
    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 35\r\nConnection: keep-alive\r\n\r\n{\"approved\":true,\"fraud_score\":0.4}",
    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 36\r\nConnection: keep-alive\r\n\r\n{\"approved\":false,\"fraud_score\":0.6}",
    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 36\r\nConnection: keep-alive\r\n\r\n{\"approved\":false,\"fraud_score\":0.8}",
    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 34\r\nConnection: keep-alive\r\n\r\n{\"approved\":false,\"fraud_score\":1}",
];

struct State {
    vectors: data::Vectors,
    labels: Mmap,
    nodes: Vec<search::Node>,
    norm: vector::Normalization,
    mcc_risk: HashMap<String, f64>,
}

static STATE: OnceCell<State> = OnceCell::new();

thread_local! {
    static HEAP_BUF: RefCell<Vec<search::Result>> =
        RefCell::new(Vec::with_capacity(search::KNN));
    static OUT_BUF: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(8192));
    static FD_BUFFERS: RefCell<HashMap<i64, Vec<u8>>> = RefCell::new(HashMap::with_capacity(256));
}

fn load_state() -> Result<State, String> {
    let dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "/data".to_string());
    let dir = Path::new(&dir);
    let vectors = data::load_vectors(dir.join("vectors.i16").to_str().unwrap())
        .map_err(|e| format!("vectors: {}", e))?;
    let labels = data::load_labels(dir.join("labels.u8").to_str().unwrap(), vectors.count)
        .map_err(|e| format!("labels: {}", e))?;
    let (nodes, _) = search::load_vptree(dir.join("vptree.bin").to_str().unwrap(), vectors.count)
        .map_err(|e| format!("vptree: {}", e))?;
    let norm: vector::Normalization = serde_json::from_str(NORMALIZATION_JSON)
        .map_err(|e| format!("normalization: {}", e))?;
    let mcc_risk: HashMap<String, f64> = serde_json::from_str(MCC_RISK_JSON)
        .map_err(|e| format!("mcc_risk: {}", e))?;
    Ok(State { vectors, labels, nodes, norm, mcc_risk })
}

fn warmup_state(state: &State) {
    let mut sum: u64 = 0;
    let body = &state.vectors.mmap[state.vectors.payload_offset..];
    let mut i = 0;
    while i < body.len() { sum = sum.wrapping_add(body[i] as u64); i += 4096; }
    let labels = &state.labels[..];
    let mut i = 0;
    while i < labels.len() { sum = sum.wrapping_add(labels[i] as u64); i += 4096; }
    std::hint::black_box(sum);

    let mut state_lcg: u32 = 0x12345678;
    let mut q = [0i16; 16];
    for _ in 0..500 {
        for slot in q.iter_mut().take(14) {
            state_lcg = state_lcg.wrapping_mul(1664525).wrapping_add(1013904223);
            *slot = ((state_lcg >> 16) as i16).rem_euclid(10001);
        }
        let _ = search::search_vptree(&state.nodes, body, q.as_ptr());
    }
}

extern "C" fn module_startup(_ty: i32, _mod_num: i32) -> i32 {
    let t0 = std::time::Instant::now();
    let state = match load_state() {
        Ok(s) => s,
        Err(e) => { eprintln!("rinha: load_state failed: {}", e); return 1; }
    };
    let t_load = t0.elapsed();
    let t1 = std::time::Instant::now();
    warmup_state(&state);
    let t_warm = t1.elapsed();
    let _ = STATE.set(state);
    eprintln!("rinha: load {:?}, warmup {:?}", t_load, t_warm);
    0
}

#[inline]
fn find_header_end(buf: &[u8]) -> Option<usize> {
    if buf.len() < 4 { return None; }
    let mut i = 0;
    let end = buf.len() - 3;
    while i < end {
        if buf[i] == b'\r' && buf[i+1] == b'\n' && buf[i+2] == b'\r' && buf[i+3] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[inline]
fn parse_content_length(headers: &[u8]) -> usize {
    const TAG: &[u8; 15] = b"content-length:";
    if headers.len() < 15 { return 0; }
    let limit = headers.len() - 15;
    let mut i = 0;
    while i <= limit {
        let c = headers[i] | 0x20;
        if c == b'c' {
            let mut equal = true;
            let mut j = 1;
            while j < 15 {
                if (headers[i + j] | 0x20) != TAG[j] {
                    equal = false;
                    break;
                }
                j += 1;
            }
            if equal {
                let mut p = i + 15;
                while p < headers.len() && (headers[p] == b' ' || headers[p] == b'\t') {
                    p += 1;
                }
                let mut v = 0usize;
                while p < headers.len() && headers[p].is_ascii_digit() {
                    v = v.wrapping_mul(10).wrapping_add((headers[p] - b'0') as usize);
                    p += 1;
                }
                return v;
            }
        }
        i += 1;
    }
    0
}

enum Parsed {
    Incomplete,
    Bad,
    Ready(usize),
    NotFound(usize),
    Fraud(usize, usize, usize),
}

#[inline]
fn parse_one(buf: &[u8]) -> Parsed {
    if buf.len() < 16 { return Parsed::Incomplete; }
    let header_end = match find_header_end(buf) {
        Some(p) => p,
        None => return Parsed::Incomplete,
    };

    let mut line_end = 0;
    while line_end < header_end {
        if buf[line_end] == b'\r' { break; }
        line_end += 1;
    }
    if line_end == 0 || line_end == header_end { return Parsed::Bad; }
    let line = &buf[..line_end];

    if line.len() >= 5 && &line[0..5] == b"POST " {
        let rest = &line[5..];
        if path_eq(rest, b"/fraud-score") {
            let cl = parse_content_length(&buf[line_end..header_end]);
            let body_start = header_end + 4;
            let body_end = body_start + cl;
            if buf.len() < body_end { return Parsed::Incomplete; }
            return Parsed::Fraud(body_start, body_end, body_end);
        }
        return Parsed::NotFound(header_end + 4);
    }

    if line.len() >= 4 && &line[0..4] == b"GET " {
        let rest = &line[4..];
        if path_eq(rest, b"/ready") {
            return Parsed::Ready(header_end + 4);
        }
        return Parsed::NotFound(header_end + 4);
    }

    Parsed::Bad
}

#[inline(always)]
fn path_eq(rest: &[u8], path: &[u8]) -> bool {
    if rest.len() < path.len() + 1 { return false; }
    if &rest[..path.len()] != path { return false; }
    let next = rest[path.len()];
    next == b' ' || next == b'?'
}

#[inline]
fn process_fraud(body: &[u8], state: &State) -> &'static [u8] {
    let count = match vector::quantize_payload(body, &state.norm, &state.mcc_risk) {
        Ok(q) => {
            let vec_payload = &state.vectors.mmap[state.vectors.payload_offset..];
            let labels = &state.labels[..];
            HEAP_BUF.with(|buf| {
                let mut heap = buf.borrow_mut();
                heap.clear();
                search::search_into(&state.nodes, vec_payload, q.as_ptr(), &mut heap);
                heap.iter().filter(|r| labels[r.index as usize] == 1).count()
            })
        }
        Err(_) => 0,
    };
    FRAUD_RESPONSES[count.min(5)]
}

fn handle_batch(fd: i64, new_bytes: &[u8]) -> Vec<u8> {
    let state = match STATE.get() {
        Some(s) => s,
        None => return ERROR_500.to_vec(),
    };

    OUT_BUF.with(|out_cell| {
        FD_BUFFERS.with(|map_cell| {
            let mut out = out_cell.borrow_mut();
            out.clear();

            let mut map = map_cell.borrow_mut();
            let entry = map.entry(fd).or_insert_with(|| Vec::with_capacity(4096));
            entry.extend_from_slice(new_bytes);

            let mut head = 0usize;
            loop {
                let buf = &entry[head..];
                if buf.is_empty() { break; }
                match parse_one(buf) {
                    Parsed::Incomplete => break,
                    Parsed::Bad => {
                        out.extend_from_slice(BAD_REQUEST);
                        head = entry.len();
                        break;
                    }
                    Parsed::Ready(c) => {
                        out.extend_from_slice(READY_RESPONSE);
                        head += c;
                    }
                    Parsed::NotFound(c) => {
                        out.extend_from_slice(NOT_FOUND);
                        head += c;
                    }
                    Parsed::Fraud(bs, be, c) => {
                        let body = &buf[bs..be];
                        let resp = process_fraud(body, state);
                        out.extend_from_slice(resp);
                        head += c;
                    }
                }
            }

            if head > 0 {
                if head == entry.len() {
                    entry.clear();
                } else {
                    entry.drain(..head);
                }
            }

            out.clone()
        })
    })
}

#[php_function]
pub fn rinha_handle_batch(fd: i64, payload: String) -> String {
    let bytes = handle_batch(fd, payload.as_bytes());
    unsafe { String::from_utf8_unchecked(bytes) }
}

#[php_function]
pub fn rinha_close(fd: i64) {
    FD_BUFFERS.with(|m| { m.borrow_mut().remove(&fd); });
}

#[php_module]
pub fn module(module: ModuleBuilder) -> ModuleBuilder {
    module.startup_function(module_startup)
}

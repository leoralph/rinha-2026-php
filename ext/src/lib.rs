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

// Respostas HTTP/1.1 pré-renderizadas com headers + body. Connection: keep-alive
// permite HAProxy reusar o socket pra próxima request sem reabrir.
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

// Heap pré-alocado por worker thread, reutilizado entre requests pra zero alloc.
thread_local! {
    static HEAP_BUF: RefCell<Vec<search::Result>> =
        RefCell::new(Vec::with_capacity(search::KNN));
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
    // 1) Page-fault todos os mmaps (1 byte por página).
    let mut sum: u64 = 0;
    let body = &state.vectors.mmap[state.vectors.payload_offset..];
    let mut i = 0;
    while i < body.len() { sum = sum.wrapping_add(body[i] as u64); i += 4096; }
    let labels = &state.labels[..];
    let mut i = 0;
    while i < labels.len() { sum = sum.wrapping_add(labels[i] as u64); i += 4096; }
    std::hint::black_box(sum);

    // 2) 500 queries fake pra aquecer JIT trace + branch predictor.
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
fn handle_bytes(bytes: &[u8]) -> &'static [u8] {
    let Some(state) = STATE.get() else { return ERROR_500; };

    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut req = httparse::Request::new(&mut headers);
    let body_offset = match req.parse(bytes) {
        Ok(httparse::Status::Complete(o)) => o,
        _ => return BAD_REQUEST,
    };

    let path = req.path.unwrap_or("");
    let method = req.method.unwrap_or("");

    if method.len() == 3 && method.as_bytes() == b"GET" && path == "/ready" {
        return READY_RESPONSE;
    }
    if method.len() == 4 && method.as_bytes() == b"POST" && path == "/fraud-score" {
        let body = &bytes[body_offset..];
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
        return FRAUD_RESPONSES[count.min(5)];
    }

    NOT_FOUND
}

#[php_function]
pub fn rinha_handle(payload: String) -> String {
    // SAFETY: todas as respostas HTTP que emitimos são ASCII puro.
    unsafe { String::from_utf8_unchecked(handle_bytes(payload.as_bytes()).to_vec()) }
}

#[php_module]
pub fn module(module: ModuleBuilder) -> ModuleBuilder {
    module.startup_function(module_startup)
}

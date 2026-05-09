mod data;
mod search;
mod vector;

use ext_php_rs::prelude::*;
use mimalloc::MiMalloc;
use memmap2::Mmap;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::path::Path;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const NORMALIZATION_JSON: &str = include_str!("../resources/normalization.json");
const MCC_RISK_JSON: &str = include_str!("../resources/mcc_risk.json");

struct State {
    vectors: data::Vectors,
    labels: Mmap,
    nodes: Vec<search::Node>,
    norm: vector::Normalization,
    mcc_risk: HashMap<String, f64>,
}

static STATE: OnceCell<State> = OnceCell::new();

fn load_state() -> Result<State, String> {
    let dir = std::env::var("DATA_DIR").unwrap_or_else(|_| "/data".to_string());
    let dir = Path::new(&dir);
    let vectors = data::load_vectors(dir.join("vectors.i24").to_str().unwrap())
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
    // Toca todas as páginas dos mmaps pra forçar demand-paging.
    let mut sum: u64 = 0;
    let body = &state.vectors.mmap[state.vectors.payload_offset..];
    let mut i = 0;
    while i < body.len() {
        sum = sum.wrapping_add(body[i] as u64);
        i += 4096;
    }
    let labels = &state.labels[..];
    let mut i = 0;
    while i < labels.len() {
        sum = sum.wrapping_add(labels[i] as u64);
        i += 4096;
    }
    std::hint::black_box(sum);
}

extern "C" fn module_startup(_ty: i32, _mod_num: i32) -> i32 {
    let t0 = std::time::Instant::now();
    let state = match load_state() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("rinha: load_state failed: {}", e);
            return 1;
        }
    };
    let t_load = t0.elapsed();
    let t1 = std::time::Instant::now();
    warmup_state(&state);
    let t_warm = t1.elapsed();
    let _ = STATE.set(state);
    eprintln!("rinha: load {:?}, warmup {:?}", t_load, t_warm);
    0
}

#[php_function]
pub fn rinha_fraud_count(payload: String) -> u32 {
    let Some(state) = STATE.get() else { return 0; };
    let query = match vector::quantize_payload(payload.as_bytes(), &state.norm, &state.mcc_risk) {
        Ok(q) => q,
        Err(_) => return 0,
    };
    let body = &state.vectors.mmap[state.vectors.payload_offset..];
    let results = search::search_vptree(&state.nodes, body, &query);
    let labels = &state.labels[..];
    let mut count: u32 = 0;
    for r in results.iter() {
        if labels[r.index as usize] == 1 {
            count += 1;
        }
    }
    count
}

#[php_module]
pub fn module(module: ModuleBuilder) -> ModuleBuilder {
    module.startup_function(module_startup)
}

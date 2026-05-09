mod data;
mod response;
mod search;
mod vector;

use ext_php_rs::prelude::*;
use memmap2::Mmap;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::path::Path;

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

fn ensure_loaded() -> Result<&'static State, String> {
    if let Some(s) = STATE.get() {
        return Ok(s);
    }
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
    let _ = STATE.set(State { vectors, labels, nodes, norm, mcc_risk });
    Ok(STATE.get().unwrap())
}

#[php_function]
pub fn rinha_warmup() -> bool {
    let state = match ensure_loaded() {
        Ok(s) => s,
        Err(_) => return false,
    };
    // Toca todas as páginas dos mmaps pra forçar demand-paging cold → hot
    // antes do /ready responder 200. Sem isso, a primeira leva da Rinha paga
    // ~32k page faults que aparecem como 200ms+ na cauda.
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
    true
}

#[php_function]
pub fn rinha_score(payload: String) -> String {
    let state = match ensure_loaded() {
        Ok(s) => s,
        Err(e) => return format!(r#"{{"error":"{}"}}"#, e),
    };
    let query = match vector::quantize_payload(payload.as_bytes(), &state.norm, &state.mcc_risk) {
        Ok(q) => q,
        Err(e) => return format!(r#"{{"error":"{}"}}"#, e),
    };
    let payload_off = state.vectors.payload_offset;
    let body = &state.vectors.mmap[payload_off..];
    let results = search::search_vptree(&state.nodes, body, &query);
    let labels = &state.labels[..];
    let fraud_count = results.iter().filter(|r| labels[r.index as usize] == 1).count();
    response::FRAUD_BODIES[fraud_count.min(5)].to_string()
}

#[php_module]
pub fn module(module: ModuleBuilder) -> ModuleBuilder {
    module
}

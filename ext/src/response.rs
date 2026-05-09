// Pre-rendered respostas (count fraud em top-5 → string JSON).
pub const FRAUD_BODIES: [&str; 6] = [
    r#"{"approved":true,"fraud_score":0}"#,
    r#"{"approved":true,"fraud_score":0.2}"#,
    r#"{"approved":true,"fraud_score":0.4}"#,
    r#"{"approved":false,"fraud_score":0.6}"#,
    r#"{"approved":false,"fraud_score":0.8}"#,
    r#"{"approved":false,"fraud_score":1}"#,
];

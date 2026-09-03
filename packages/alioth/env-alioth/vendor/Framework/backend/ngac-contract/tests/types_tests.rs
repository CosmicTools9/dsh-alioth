use ngac_contract::*;

#[test]
fn test_decision_display() {
    assert_eq!(Decision::Permit.to_string(), "Permit");
    assert_eq!(Decision::Deny.to_string(), "Deny");
    assert_eq!(Decision::NotApplicable.to_string(), "NotApplicable");
}

#[test]
fn test_decision_serde_roundtrip() {
    let cases = vec![Decision::Permit, Decision::Deny, Decision::NotApplicable];
    for d in cases {
        let json = serde_json::to_string(&d).unwrap();
        let back: Decision = serde_json::from_str(&json).unwrap();
        assert_eq!(d, back);
    }
}

#[test]
fn test_pdp_check_request_serde() {
    let req = PdpCheckRequest {
        user_id: 42,
        resource: "isahl.zc_id_production".to_string(),
        action: "read".to_string(),
    };
    let json = serde_json::to_string(&req).unwrap();
    let back: PdpCheckRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.user_id, 42);
    assert_eq!(back.resource, "isahl.zc_id_production");
}

#[test]
fn test_pdp_check_response_serde() {
    let resp = PdpCheckResponse {
        permitted: true,
        reason: "Owner access".to_string(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let back: PdpCheckResponse = serde_json::from_str(&json).unwrap();
    assert!(back.permitted);
}

#[test]
fn test_pdp_check_batch_serde() {
    let batch = PdpCheckBatchRequest {
        user_id: 42,
        checks: vec![
            CheckItem {
                resource: "a".to_string(),
                action: "read".to_string(),
            },
            CheckItem {
                resource: "b".to_string(),
                action: "write".to_string(),
            },
        ],
    };
    let json = serde_json::to_string(&batch).unwrap();
    let back: PdpCheckBatchRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.checks.len(), 2);
}

#[test]
fn test_pdp_check_batch_response_serde() {
    let resp = PdpCheckBatchResponse {
        results: vec![PdpCheckResponse {
            permitted: true,
            reason: "ok".to_string(),
        }],
    };
    let json = serde_json::to_string(&resp).unwrap();
    let back: PdpCheckBatchResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(back.results.len(), 1);
}

#[test]
fn test_ngac_error_display() {
    let err = NgacError::HttpError("connection refused".to_string());
    assert!(err.to_string().contains("connection refused"));

    let err = NgacError::ServiceUnavailable("SSO down".to_string());
    assert!(err.to_string().contains("SSO down"));

    let err = NgacError::InvalidResponse("bad json".to_string());
    assert!(err.to_string().contains("bad json"));
}

#[test]
fn test_http_ngac_client_construction() {
    let client = HttpNgacClient::new("http://localhost:9002");
    // just verify it doesn't panic and is Debug
    let _ = format!("{:?}", client);
}

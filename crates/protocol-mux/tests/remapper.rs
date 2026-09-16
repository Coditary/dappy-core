use protocol_mux::{BackendSeq, ClientSeq, MessageRemapper, request, translate_cancel};

#[test]
fn translate_cancel_known_request_id() {
    let remapper = MessageRemapper::new();
    remapper.map(ClientSeq::new(10), BackendSeq::new(99));

    let mut cancel = request(11, "cancel", Some(serde_json::json!({ "requestId": 10 })));
    assert!(translate_cancel(&mut cancel, &remapper));
    assert_eq!(cancel["arguments"]["requestId"], 99);
}

#[test]
fn translate_cancel_unknown_request_id_preserved() {
    let remapper = MessageRemapper::new();
    let mut cancel = request(11, "cancel", Some(serde_json::json!({ "requestId": 10 })));
    assert!(!translate_cancel(&mut cancel, &remapper));
    assert_eq!(cancel["arguments"]["requestId"], 10);
}

#[test]
fn translate_cancel_before_own_mapping() {
    let remapper = MessageRemapper::new();
    let mut cancel = request(5, "cancel", Some(serde_json::json!({ "requestId": 5 })));
    assert!(!translate_cancel(&mut cancel, &remapper));
    assert_eq!(cancel["arguments"]["requestId"], 5);
}

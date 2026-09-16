use dap_protocol::{Message, Request, read_message, write_message};
use tokio::io::BufReader;

#[tokio::test]
async fn roundtrip_write_request() {
    let msg = Message::Request(Request {
        seq: 1,
        command: "initialize".into(),
        arguments: None,
    });

    let mut buf = Vec::new();
    write_message(&mut buf, &msg).await.unwrap();

    assert!(buf.starts_with(b"Content-Length:"));
    let body = String::from_utf8_lossy(&buf);
    assert!(body.contains("initialize"));
}

#[tokio::test]
async fn roundtrip_read_request() {
    let body = r#"{"type":"request","seq":1,"command":"initialize","arguments":{}}"#;
    let payload = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let mut reader = BufReader::new(payload.as_bytes());
    let msg = read_message(&mut reader).await.unwrap().expect("message");
    assert!(matches!(msg, Message::Request(req) if req.command == "initialize"));
}

#[tokio::test]
async fn read_debugpy_style_response() {
    let body = r#"{"seq":4,"type":"response","request_seq":1,"command":"initialize","success":true,"body":{}}"#;
    let payload = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
    let mut reader = BufReader::new(payload.as_bytes());
    let msg = read_message(&mut reader).await.unwrap().expect("message");
    match msg {
        Message::Response(resp) => {
            assert_eq!(resp.request_seq, 1);
            assert!(resp.success);
        }
        other => panic!("expected response, got {other:?}"),
    }
}

#[tokio::test]
async fn response_serializes_request_seq_once() {
    use dap_protocol::{Message, Response};
    let msg = Message::Response(Response {
        seq: 2,
        request_seq: 1,
        success: true,
        command: Some("initialize".into()),
        message: None,
        body: Some(serde_json::json!({})),
    });
    let json = serde_json::to_string(&msg).unwrap();
    assert_eq!(json.matches("request_seq").count(), 1);
    assert_eq!(json.matches("requestSeq").count(), 0);
}

#[tokio::test]
async fn read_eof_returns_none() {
    let mut reader = BufReader::new(&[][..]);
    let msg = read_message(&mut reader).await.unwrap();
    assert!(msg.is_none());
}
